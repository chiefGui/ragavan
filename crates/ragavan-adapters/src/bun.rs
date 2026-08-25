use crate::ResolvedScript;
use std::{
    env,
    ffi::OsString,
    fmt, fs, io,
    path::{Path, PathBuf},
};

pub struct BunDev<'a> {
    arguments: &'a [OsString],
    script_index: usize,
}

impl<'a> BunDev<'a> {
    pub fn recognize(arguments: &'a [OsString]) -> Option<Self> {
        let script_index = match arguments {
            [script, ..] if script == "dev" => 0,
            [run, script, ..] if run == "run" && script == "dev" => 1,
            _ => return None,
        };

        Some(Self {
            arguments,
            script_index,
        })
    }

    pub(crate) fn script_arguments(&self) -> &'a [OsString] {
        &self.arguments[self.script_index + 1..]
    }

    pub(crate) fn resolve(self, worktree_root: &Path) -> Result<ResolvedScript, Error> {
        let package_path = nearest_package_json(worktree_root)?;
        let package = fs::read(&package_path).map_err(|source| Error::ReadPackage {
            path: package_path.clone(),
            source,
        })?;
        let package: serde_json::Value =
            serde_json::from_slice(&package).map_err(|source| Error::ParsePackage {
                path: package_path.clone(),
                source,
            })?;
        let command = package
            .get("scripts")
            .and_then(|scripts| scripts.get("dev"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| Error::MissingDevScript {
                path: package_path.clone(),
            })?
            .to_owned();

        Ok(ResolvedScript {
            package_path,
            command,
        })
    }
}

fn nearest_package_json(worktree_root: &Path) -> Result<PathBuf, Error> {
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
            current_directory,
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

#[derive(Debug)]
pub(crate) enum Error {
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
    MissingDevScript {
        path: PathBuf,
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
                "could not isolate `bun dev`: no package.json exists between the current directory and {}",
                root.display()
            ),
            Self::ReadPackage { path, source } => {
                write!(formatter, "could not read {}: {source}", path.display())
            }
            Self::ParsePackage { path, source } => {
                write!(formatter, "could not parse {}: {source}", path.display())
            }
            Self::MissingDevScript { path } => write!(
                formatter,
                "could not isolate `bun dev`: {} has no string `scripts.dev`",
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
            _ => None,
        }
    }
}
