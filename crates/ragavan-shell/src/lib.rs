#![forbid(unsafe_code)]

mod powershell;

use std::{
    fmt,
    path::{Path, PathBuf},
};

pub mod protocol {
    pub const BUN_ARGUMENTS_COMMAND: &str = "__bun-arguments";
    pub const PASSTHROUGH_EXIT_CODE: u8 = 10;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellTarget {
    Current,
    PowerShell,
}

#[derive(Debug, Eq, PartialEq)]
pub enum InstallOutcome {
    Installed { profile: PathBuf },
    AlreadyInstalled { profile: PathBuf },
}

impl InstallOutcome {
    pub fn profile(&self) -> &Path {
        match self {
            Self::Installed { profile } | Self::AlreadyInstalled { profile } => profile,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum UninstallOutcome {
    Uninstalled { profile: PathBuf },
    AlreadyUninstalled { profile: PathBuf },
}

impl UninstallOutcome {
    pub fn profile(&self) -> &Path {
        match self {
            Self::Uninstalled { profile } | Self::AlreadyUninstalled { profile } => profile,
        }
    }
}

pub fn install(target: ShellTarget) -> Result<InstallOutcome, Error> {
    resolve(target)?.install().map_err(Error::powershell)
}

pub fn uninstall(target: ShellTarget) -> Result<UninstallOutcome, Error> {
    resolve(target)?.uninstall().map_err(Error::powershell)
}

pub fn powershell_hook() -> String {
    powershell::hook()
}

fn resolve(target: ShellTarget) -> Result<powershell::PowerShell, Error> {
    match target {
        ShellTarget::Current => powershell::PowerShell::current()
            .map_err(Error::powershell)?
            .ok_or_else(Error::unsupported_current_shell),
        ShellTarget::PowerShell => powershell::PowerShell::available().map_err(Error::powershell),
    }
}

#[derive(Debug)]
pub struct Error(ErrorKind);

#[derive(Debug)]
enum ErrorKind {
    UnsupportedCurrentShell,
    PowerShell(powershell::Error),
}

impl Error {
    fn unsupported_current_shell() -> Self {
        Self(ErrorKind::UnsupportedCurrentShell)
    }

    fn powershell(error: powershell::Error) -> Self {
        Self(ErrorKind::PowerShell(error))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            ErrorKind::UnsupportedCurrentShell => formatter.write_str(
                "could not detect a supported current shell; rerun the command with `powershell`",
            ),
            ErrorKind::PowerShell(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.0 {
            ErrorKind::UnsupportedCurrentShell => None,
            ErrorKind::PowerShell(error) => Some(error),
        }
    }
}
