#![forbid(unsafe_code)]

mod bun;
mod vite;

pub use bun::BunDev;
pub use vite::ViteDev;

use std::{
    fmt,
    path::{Path, PathBuf},
};

pub fn recognize_vite(bun_dev: BunDev<'_>, worktree_root: &Path) -> Result<ViteDev, Error> {
    vite::reject_explicit_port(bun_dev.script_arguments())?;
    let script = bun_dev.resolve(worktree_root)?;
    Ok(vite::recognize(script)?)
}

struct ResolvedScript {
    package_path: PathBuf,
    command: String,
}

impl ResolvedScript {
    fn package_path(&self) -> &Path {
        &self.package_path
    }

    fn command(&self) -> &str {
        &self.command
    }
}

#[derive(Debug)]
pub struct Error(ErrorKind);

#[derive(Debug)]
enum ErrorKind {
    Bun(bun::Error),
    Vite(vite::Error),
}

impl From<bun::Error> for Error {
    fn from(error: bun::Error) -> Self {
        Self(ErrorKind::Bun(error))
    }
}

impl From<vite::Error> for Error {
    fn from(error: vite::Error) -> Self {
        Self(ErrorKind::Vite(error))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            ErrorKind::Bun(error) => error.fmt(formatter),
            ErrorKind::Vite(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.0 {
            ErrorKind::Bun(error) => Some(error),
            ErrorKind::Vite(error) => Some(error),
        }
    }
}
