use ragavan_core::{ServiceScope, ServiceScopeError};
use std::{
    env, fmt, fs, io,
    path::{Path, PathBuf},
};

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

pub(super) fn find_script(worktree_root: &Path, script_name: &str) -> Result<PackageScript, Error> {
    let (package_path, scope) = nearest_package_json(worktree_root)?;
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

fn nearest_package_json(worktree_root: &Path) -> Result<(PathBuf, ServiceScope), Error> {
    let current_directory = env::current_dir().map_err(Error::CurrentDirectory)?;
    let resolved_current_directory =
        fs::canonicalize(&current_directory).map_err(|source| Error::ResolveDirectory {
            path: current_directory.clone(),
            source,
        })?;
    let resolved_worktree_root =
        fs::canonicalize(worktree_root).map_err(|source| Error::ResolveDirectory {
            path: worktree_root.to_owned(),
            source,
        })?;

    if !resolved_current_directory.starts_with(&resolved_worktree_root) {
        return Err(Error::CurrentDirectoryOutsideWorktree {
            current_directory: current_directory.clone(),
            root: worktree_root.to_owned(),
        });
    }

    for directory in resolved_current_directory.ancestors() {
        let package_path = directory.join("package.json");
        match fs::metadata(&package_path) {
            Ok(metadata) if metadata.is_file() => {
                let relative_directory =
                    directory
                        .strip_prefix(&resolved_worktree_root)
                        .map_err(|_| Error::CurrentDirectoryOutsideWorktree {
                            current_directory: current_directory.clone(),
                            root: worktree_root.to_owned(),
                        })?;
                let scope =
                    ServiceScope::from_relative_path(relative_directory).map_err(|source| {
                        Error::InvalidServiceScope {
                            path: directory.to_owned(),
                            source,
                        }
                    })?;
                return Ok((package_path, scope));
            }
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

#[derive(Debug)]
pub(super) enum Error {
    CurrentDirectory(io::Error),
    ResolveDirectory {
        path: PathBuf,
        source: io::Error,
    },
    CurrentDirectoryOutsideWorktree {
        current_directory: PathBuf,
        root: PathBuf,
    },
    MissingPackage {
        root: PathBuf,
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
            Self::CurrentDirectory(source) => {
                write!(formatter, "could not read the current directory: {source}")
            }
            Self::ResolveDirectory { path, source } => {
                write!(
                    formatter,
                    "could not resolve directory {}: {source}",
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
            Self::MissingPackage { root } => write!(
                formatter,
                "no package.json exists between the current directory and {}",
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
            Self::ResolveDirectory { source, .. } => Some(source),
            Self::ReadPackage { source, .. } => Some(source),
            Self::ParsePackage { source, .. } => Some(source),
            Self::InvalidServiceScope { source, .. } => Some(source),
            _ => None,
        }
    }
}
