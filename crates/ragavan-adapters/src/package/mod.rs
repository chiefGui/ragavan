mod target;

pub(super) use target::{PackageSelector, PackageTarget, SelectorBase};

use ragavan_core::{ServiceScope, ServiceScopeError};
use ragavan_diagnostics::{Detail, Diagnostic};
use std::{
    env,
    ffi::{OsStr, OsString},
    fmt, fs, io,
    path::{Path, PathBuf},
};

pub(super) fn find_script(
    worktree_root: &Path,
    target: PackageTarget<'_>,
    script_name: &str,
) -> Result<PackageScript, Error> {
    match target {
        PackageTarget::CurrentDirectory => find_nearest_script(worktree_root, script_name),
        PackageTarget::Selected(selector) => {
            find_selected_script(worktree_root, selector, script_name)
        }
        PackageTarget::MissingValue(option) => Err(Error::MissingTargetValue(option)),
        PackageTarget::Multiple => Err(Error::MultipleTargets),
        PackageTarget::NonExact(selector) => Err(Error::NonExactTarget(selector.to_owned())),
    }
}

pub(super) struct PackageScript {
    path: PathBuf,
    scope: ServiceScope,
    source: String,
}

impl PackageScript {
    pub(super) fn into_parts(self) -> (PathBuf, ServiceScope, String) {
        (self.path, self.scope, self.source)
    }
}

fn find_nearest_script(worktree_root: &Path, script_name: &str) -> Result<PackageScript, Error> {
    let resolved_worktree_root = resolve_path(worktree_root)?;
    let package_path = nearest_package_json(worktree_root, &resolved_worktree_root)?;
    read_script(&resolved_worktree_root, package_path, script_name)
}

fn find_selected_script(
    worktree_root: &Path,
    selector: PackageSelector<'_>,
    script_name: &str,
) -> Result<PackageScript, Error> {
    let resolved_worktree_root = resolve_path(worktree_root)?;
    let package_path = selected_package_json(&resolved_worktree_root, selector)?;
    read_script(&resolved_worktree_root, package_path, script_name)
}

fn read_script(
    resolved_worktree_root: &Path,
    package_path: PathBuf,
    script_name: &str,
) -> Result<PackageScript, Error> {
    let package_path = resolve_package_path(&package_path)?;
    let package_directory = package_path
        .parent()
        .expect("a package manifest discovered by Ragavan always has a parent directory");
    let relative_directory = package_directory
        .strip_prefix(resolved_worktree_root)
        .map_err(|_| Error::PackageOutsideWorktree {
            package: package_directory.to_owned(),
            root: resolved_worktree_root.to_owned(),
        })?;
    let scope = ServiceScope::from_relative_path(relative_directory).map_err(|source| {
        Error::InvalidServiceScope {
            path: package_directory.to_owned(),
            source,
        }
    })?;
    let package = fs::read(&package_path).map_err(|source| Error::ReadPackage {
        path: package_path.clone(),
        source,
    })?;
    let mut package: serde_json::Value =
        serde_json::from_slice(&package).map_err(|source| Error::ParsePackage {
            path: package_path.clone(),
            source,
        })?;
    let Some(serde_json::Value::String(source)) = package
        .get_mut("scripts")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|scripts| scripts.remove(script_name))
    else {
        return Err(Error::MissingScript {
            path: package_path,
            name: script_name.to_owned(),
        });
    };

    Ok(PackageScript {
        path: package_path,
        scope,
        source,
    })
}

fn resolve_path(path: &Path) -> Result<PathBuf, Error> {
    fs::canonicalize(path).map_err(|source| Error::ResolvePath {
        path: path.to_owned(),
        source,
    })
}

fn resolve_package_path(path: &Path) -> Result<PathBuf, Error> {
    let file_name = path
        .file_name()
        .expect("a package manifest discovered by Ragavan always has a filename");
    let directory = path
        .parent()
        .expect("a package manifest discovered by Ragavan always has a parent directory");
    Ok(resolve_path(directory)?.join(file_name))
}

fn nearest_package_json(
    worktree_root: &Path,
    resolved_worktree_root: &Path,
) -> Result<PathBuf, Error> {
    let current_directory = env::current_dir().map_err(Error::CurrentDirectory)?;
    let resolved_current_directory = resolve_path(&current_directory)?;

    if !resolved_current_directory.starts_with(resolved_worktree_root) {
        return Err(Error::CurrentDirectoryOutsideWorktree {
            current_directory: current_directory.clone(),
            root: worktree_root.to_owned(),
        });
    }

    for directory in resolved_current_directory.ancestors() {
        let package_path = directory.join("package.json");
        match fs::metadata(&package_path) {
            Ok(metadata) if metadata.is_file() => return Ok(package_path),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(Error::ReadPackage {
                    path: package_path,
                    source,
                });
            }
        }

        if directory == resolved_worktree_root {
            return Err(Error::MissingPackage {
                root: worktree_root.to_owned(),
            });
        }
    }

    Err(Error::CurrentDirectoryOutsideWorktree {
        current_directory,
        root: worktree_root.to_owned(),
    })
}

fn selected_package_json(
    resolved_worktree_root: &Path,
    selector: PackageSelector<'_>,
) -> Result<PathBuf, Error> {
    let value = selector.value();
    let packages = match selector {
        PackageSelector::Name(name) => {
            let Some(name) = name.to_str() else {
                return Err(Error::MissingSelectedPackage {
                    selector: name.to_owned(),
                    root: resolved_worktree_root.to_owned(),
                });
            };
            packages_below(resolved_worktree_root, resolved_worktree_root, Some(name))?
        }
        PackageSelector::Directory { value, relative_to } => {
            let base = selector_base(resolved_worktree_root, relative_to)?;
            selected_directory_packages(
                resolved_worktree_root,
                &base,
                value,
                DirectoryTraversal::StopAtPackage,
            )?
        }
        PackageSelector::NameOrDirectory { value, relative_to } => {
            let base = selector_base(resolved_worktree_root, relative_to)?;
            let mut packages = selected_directory_packages(
                resolved_worktree_root,
                &base,
                value,
                DirectoryTraversal::IncludeDescendants,
            )?;
            if let Some(name) = value.to_str() {
                for package in
                    packages_below(resolved_worktree_root, resolved_worktree_root, Some(name))?
                {
                    if !packages.contains(&package) {
                        packages.push(package);
                        if packages.len() == 2 {
                            break;
                        }
                    }
                }
            }
            packages
        }
    };

    exactly_one_package(packages, resolved_worktree_root, value)
}

fn selector_base(root: &Path, base: SelectorBase) -> Result<PathBuf, Error> {
    match base {
        SelectorBase::CurrentDirectory => {
            let current_directory = env::current_dir().map_err(Error::CurrentDirectory)?;
            resolve_path(&current_directory)
        }
        SelectorBase::WorktreeRoot => Ok(root.to_owned()),
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum DirectoryTraversal {
    StopAtPackage,
    IncludeDescendants,
}

fn selected_directory_packages(
    resolved_worktree_root: &Path,
    relative_to: &Path,
    selector: &OsStr,
    traversal: DirectoryTraversal,
) -> Result<Vec<PathBuf>, Error> {
    let selector_path = Path::new(selector);
    let candidate = if selector_path.is_absolute() {
        selector_path.to_owned()
    } else {
        relative_to.join(selector_path)
    };

    match fs::canonicalize(&candidate) {
        Ok(candidate) => {
            if !candidate.starts_with(resolved_worktree_root) {
                return Err(Error::PackageTargetOutsideWorktree {
                    selector: selector.to_owned(),
                    root: resolved_worktree_root.to_owned(),
                });
            }
            if fs::metadata(&candidate)
                .map_err(|source| Error::ResolvePath {
                    path: candidate.clone(),
                    source,
                })?
                .is_dir()
            {
                let direct_package = candidate.join("package.json");
                let direct_package = match fs::metadata(&direct_package) {
                    Ok(metadata) if metadata.is_file() => Some(direct_package),
                    Ok(_) => None,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                    Err(source) => {
                        return Err(Error::ReadPackage {
                            path: direct_package,
                            source,
                        });
                    }
                };
                if let Some(direct_package) = direct_package.as_ref()
                    && traversal == DirectoryTraversal::StopAtPackage
                {
                    return Ok(vec![direct_package.to_owned()]);
                }

                let mut packages = packages_below(resolved_worktree_root, &candidate, None)?;
                if let Some(direct_package) = direct_package
                    && !packages.contains(&direct_package)
                {
                    packages.insert(0, direct_package);
                    packages.truncate(2);
                }
                return Ok(packages);
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(Error::ResolvePackageTarget {
                selector: selector.to_owned(),
                source,
            });
        }
    }

    Ok(Vec::new())
}

fn exactly_one_package(
    packages: Vec<PathBuf>,
    root: &Path,
    selector: &OsStr,
) -> Result<PathBuf, Error> {
    match packages.as_slice() {
        [package] => Ok(package.clone()),
        [] => Err(Error::MissingSelectedPackage {
            selector: selector.to_owned(),
            root: root.to_owned(),
        }),
        [_, _, ..] => Err(Error::AmbiguousSelectedPackage {
            selector: selector.to_owned(),
        }),
    }
}

fn packages_below(
    root: &Path,
    directory: &Path,
    selected_name: Option<&str>,
) -> Result<Vec<PathBuf>, Error> {
    let mut packages = Vec::with_capacity(2);
    let manifests = ragavan_git::source_files_named(root, "package.json").map_err(|source| {
        Error::DiscoverPackages {
            root: root.to_owned(),
            source,
        }
    })?;

    for manifest in manifests {
        let package_path = root.join(manifest);
        match fs::metadata(&package_path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(source) => {
                return Err(Error::ReadPackage {
                    path: package_path,
                    source,
                });
            }
        }
        let package_path = resolve_package_path(&package_path)?;
        if !package_path.starts_with(root) {
            return Err(Error::PackageOutsideWorktree {
                package: package_path,
                root: root.to_owned(),
            });
        }
        if !package_path.starts_with(directory) {
            continue;
        }
        let selected = match selected_name {
            None => true,
            Some(name) => package_has_name(&package_path, name)?,
        };
        if selected && !packages.contains(&package_path) {
            packages.push(package_path);
            if packages.len() == 2 {
                break;
            }
        }
    }

    Ok(packages)
}

fn package_has_name(path: &Path, selected_name: &str) -> Result<bool, Error> {
    let package = fs::read(path).map_err(|source| Error::ReadPackage {
        path: path.to_owned(),
        source,
    })?;
    let Ok(package): Result<serde_json::Value, _> = serde_json::from_slice(&package) else {
        return Ok(false);
    };
    Ok(package.get("name").and_then(serde_json::Value::as_str) == Some(selected_name))
}

#[derive(Debug)]
pub(super) enum Error {
    MissingTargetValue(&'static str),
    MultipleTargets,
    NonExactTarget(OsString),
    CurrentDirectory(io::Error),
    ResolvePath {
        path: PathBuf,
        source: io::Error,
    },
    CurrentDirectoryOutsideWorktree {
        current_directory: PathBuf,
        root: PathBuf,
    },
    PackageOutsideWorktree {
        package: PathBuf,
        root: PathBuf,
    },
    PackageTargetOutsideWorktree {
        selector: OsString,
        root: PathBuf,
    },
    ResolvePackageTarget {
        selector: OsString,
        source: io::Error,
    },
    MissingPackage {
        root: PathBuf,
    },
    MissingSelectedPackage {
        selector: OsString,
        root: PathBuf,
    },
    AmbiguousSelectedPackage {
        selector: OsString,
    },
    DiscoverPackages {
        root: PathBuf,
        source: ragavan_git::Error,
    },
    ReadPackage {
        path: PathBuf,
        source: io::Error,
    },
    ParsePackage {
        path: PathBuf,
        source: serde_json::Error,
    },
    InvalidServiceScope {
        path: PathBuf,
        source: ServiceScopeError,
    },
    MissingScript {
        path: PathBuf,
        name: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTargetValue(option) => {
                write!(formatter, "`{option}` requires a package selector")
            }
            Self::MultipleTargets => formatter.write_str(
                "the command may select multiple packages; Ragavan requires exactly one package",
            ),
            Self::NonExactTarget(selector) => write!(
                formatter,
                "package selector {:?} is not an exact package name or directory; Ragavan requires exactly one package",
                selector.to_string_lossy()
            ),
            Self::CurrentDirectory(source) => {
                write!(formatter, "could not read the current directory: {source}")
            }
            Self::ResolvePath { path, source } => {
                write!(
                    formatter,
                    "could not resolve path {}: {source}",
                    path.display()
                )
            }
            Self::CurrentDirectoryOutsideWorktree {
                current_directory,
                root,
            } => write!(
                formatter,
                "current directory {} is outside Git worktree {}",
                current_directory.display(),
                root.display()
            ),
            Self::PackageOutsideWorktree { package, root } => write!(
                formatter,
                "package path {} is outside Git worktree {}",
                package.display(),
                root.display()
            ),
            Self::PackageTargetOutsideWorktree { selector, root } => write!(
                formatter,
                "package selector {:?} points outside Git worktree {}",
                selector.to_string_lossy(),
                root.display()
            ),
            Self::ResolvePackageTarget { selector, source } => write!(
                formatter,
                "could not resolve package selector {:?}: {source}",
                selector.to_string_lossy()
            ),
            Self::MissingPackage { root } => write!(
                formatter,
                "no package.json exists between the current directory and {}",
                root.display()
            ),
            Self::MissingSelectedPackage { selector, root } => write!(
                formatter,
                "package selector {:?} does not identify a package in {}",
                selector.to_string_lossy(),
                root.display()
            ),
            Self::AmbiguousSelectedPackage { selector } => write!(
                formatter,
                "package selector {:?} identifies multiple packages; Ragavan requires exactly one package",
                selector.to_string_lossy()
            ),
            Self::DiscoverPackages { root, source } => write!(
                formatter,
                "could not discover packages in {}: {source}",
                root.display()
            ),
            Self::ReadPackage { path, source } => {
                write!(formatter, "could not read {}: {source}", path.display())
            }
            Self::ParsePackage { path, source } => {
                write!(formatter, "could not parse {}: {source}", path.display())
            }
            Self::InvalidServiceScope { path, source } => write!(
                formatter,
                "could not identify package directory {} as a service: {source}",
                path.display()
            ),
            Self::MissingScript { path, name } => write!(
                formatter,
                "{} has no string `scripts.{name}`",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CurrentDirectory(source) => Some(source),
            Self::ResolvePath { source, .. } => Some(source),
            Self::ResolvePackageTarget { source, .. } => Some(source),
            Self::DiscoverPackages { source, .. } => Some(source),
            Self::ReadPackage { source, .. } => Some(source),
            Self::ParsePackage { source, .. } => Some(source),
            Self::InvalidServiceScope { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl Diagnostic for Error {
    fn code(&self) -> &'static str {
        match self {
            Self::MissingTargetValue(_) => "package.target.value_missing",
            Self::MultipleTargets => "package.target.multiple",
            Self::NonExactTarget(_) => "package.target.non_exact",
            Self::CurrentDirectory(_) => "package.current_directory.read",
            Self::ResolvePath { .. } => "package.path.resolve",
            Self::CurrentDirectoryOutsideWorktree { .. } => {
                "package.current_directory.outside_worktree"
            }
            Self::PackageOutsideWorktree { .. } => "package.path.outside_worktree",
            Self::PackageTargetOutsideWorktree { .. } => "package.target.outside_worktree",
            Self::ResolvePackageTarget { .. } => "package.target.resolve",
            Self::MissingPackage { .. } => "package.manifest.missing",
            Self::MissingSelectedPackage { .. } => "package.target.missing",
            Self::AmbiguousSelectedPackage { .. } => "package.target.ambiguous",
            Self::DiscoverPackages { source, .. } => source.code(),
            Self::ReadPackage { .. } => "package.manifest.read",
            Self::ParsePackage { .. } => "package.manifest.parse",
            Self::InvalidServiceScope { source, .. } => source.code(),
            Self::MissingScript { .. } => "package.script.missing",
        }
    }

    fn help(&self) -> Option<String> {
        match self {
            Self::MissingTargetValue(_)
            | Self::MultipleTargets
            | Self::NonExactTarget(_)
            | Self::MissingSelectedPackage { .. }
            | Self::AmbiguousSelectedPackage { .. } => {
                Some("select exactly one package by its name or directory".to_owned())
            }
            Self::CurrentDirectoryOutsideWorktree { .. }
            | Self::PackageOutsideWorktree { .. }
            | Self::PackageTargetOutsideWorktree { .. } => {
                Some("run the command for a package inside the enrolled Git worktree".to_owned())
            }
            Self::MissingPackage { .. } => Some(
                "run the command from a package directory or select one workspace package"
                    .to_owned(),
            ),
            Self::DiscoverPackages { source, .. } => source.help(),
            Self::ParsePackage { .. } => Some("fix the package.json syntax, then retry".to_owned()),
            Self::InvalidServiceScope { source, .. } => source.help(),
            Self::MissingScript { name, .. } => Some(format!(
                "define `scripts.{name}` as a string in package.json"
            )),
            Self::CurrentDirectory(_)
            | Self::ResolvePath { .. }
            | Self::ResolvePackageTarget { .. }
            | Self::ReadPackage { .. } => None,
        }
    }

    fn details(&self) -> Vec<Detail> {
        match self {
            Self::MissingTargetValue(option) => vec![Detail::text("option", *option)],
            Self::NonExactTarget(selector) | Self::AmbiguousSelectedPackage { selector } => {
                vec![Detail::text(
                    "selector",
                    selector.to_string_lossy().into_owned(),
                )]
            }
            Self::CurrentDirectoryOutsideWorktree {
                current_directory,
                root,
            } => vec![
                Detail::text("current_directory", current_directory.display().to_string()),
                Detail::text("worktree", root.display().to_string()),
            ],
            Self::PackageOutsideWorktree { package, root } => vec![
                Detail::text("package", package.display().to_string()),
                Detail::text("worktree", root.display().to_string()),
            ],
            Self::PackageTargetOutsideWorktree { selector, root }
            | Self::MissingSelectedPackage { selector, root } => vec![
                Detail::text("selector", selector.to_string_lossy().into_owned()),
                Detail::text("worktree", root.display().to_string()),
            ],
            Self::ResolvePackageTarget { selector, .. } => vec![Detail::text(
                "selector",
                selector.to_string_lossy().into_owned(),
            )],
            Self::MissingPackage { root } => {
                vec![Detail::text("worktree", root.display().to_string())]
            }
            Self::DiscoverPackages { root, source } => {
                let mut details = vec![Detail::text("worktree", root.display().to_string())];
                details.extend(source.details());
                details
            }
            Self::ResolvePath { path, .. }
            | Self::ReadPackage { path, .. }
            | Self::ParsePackage { path, .. } => {
                vec![Detail::text("path", path.display().to_string())]
            }
            Self::InvalidServiceScope { path, source } => {
                let mut details = vec![Detail::text("package", path.display().to_string())];
                details.extend(source.details());
                details
            }
            Self::MissingScript { path, name } => vec![
                Detail::text("package", path.display().to_string()),
                Detail::text("script", name),
            ],
            Self::MultipleTargets | Self::CurrentDirectory(_) => Vec::new(),
        }
    }
}
