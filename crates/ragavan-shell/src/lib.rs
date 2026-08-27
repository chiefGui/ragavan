#![forbid(unsafe_code)]

mod bash;
mod detection;
mod powershell;
mod profile;

use std::{
    ffi::{OsStr, OsString},
    fmt,
    path::{Path, PathBuf},
};

pub mod protocol {
    use super::{OsStr, OsString};

    pub const RUN_COMMAND: &str = "__run";

    pub struct RunRequest<'a> {
        command: &'a OsStr,
        program: &'a OsStr,
        launch_arguments: &'a [OsString],
        arguments: &'a [OsString],
    }

    impl<'a> RunRequest<'a> {
        pub fn command(&self) -> &'a OsStr {
            self.command
        }

        pub fn program(&self) -> &'a OsStr {
            self.program
        }

        pub fn launch_arguments(&self) -> &'a [OsString] {
            self.launch_arguments
        }

        pub fn arguments(&self) -> &'a [OsString] {
            self.arguments
        }
    }

    pub fn parse(arguments: &[OsString]) -> Result<Option<RunRequest<'_>>, &'static str> {
        let Some((operation, arguments)) = arguments.split_first() else {
            return Ok(None);
        };
        if operation != RUN_COMMAND {
            return Ok(None);
        }

        let [command, program, launch_argument_count, remaining @ ..] = arguments else {
            return Err("the loaded shell integration sent a malformed command to Ragavan");
        };
        let Some(launch_argument_count) = launch_argument_count
            .to_str()
            .and_then(|count| count.parse::<usize>().ok())
        else {
            return Err("the loaded shell integration sent a malformed command to Ragavan");
        };
        if command.is_empty() || program.is_empty() || launch_argument_count > remaining.len() {
            return Err("the loaded shell integration sent a malformed command to Ragavan");
        }
        let (launch_arguments, arguments) = remaining.split_at(launch_argument_count);

        Ok(Some(RunRequest {
            command,
            program,
            launch_arguments,
            arguments,
        }))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn separates_launch_arguments_from_command_arguments() {
            let arguments = [
                OsString::from(RUN_COMMAND),
                OsString::from("runner"),
                OsString::from("shell"),
                OsString::from("2"),
                OsString::from("launch-one"),
                OsString::from("launch-two"),
                OsString::from("command-one"),
                OsString::from("command-two"),
            ];

            let request = parse(&arguments)
                .expect("the protocol should be valid")
                .expect("the run command should be recognized");

            assert_eq!(request.command(), "runner");
            assert_eq!(request.program(), "shell");
            assert_eq!(request.launch_arguments(), ["launch-one", "launch-two"]);
            assert_eq!(request.arguments(), ["command-one", "command-two"]);
        }

        #[test]
        fn rejects_invalid_launch_argument_boundaries() {
            for arguments in [
                vec![RUN_COMMAND, "runner", "program", "not-a-count"],
                vec![RUN_COMMAND, "runner", "program", "1"],
            ] {
                let arguments: Vec<_> = arguments.into_iter().map(OsString::from).collect();
                assert!(parse(&arguments).is_err());
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct Shell(&'static Adapter);

impl Shell {
    pub fn name(self) -> &'static str {
        self.0.name
    }

    pub fn display_name(self) -> &'static str {
        self.0.display_name
    }

    pub fn activation_command(self) -> &'static str {
        self.0.activation_command
    }
}

impl fmt::Debug for Shell {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("Shell").field(&self.name()).finish()
    }
}

impl PartialEq for Shell {
    fn eq(&self, other: &Self) -> bool {
        self.name() == other.name()
    }
}

impl Eq for Shell {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellTarget {
    Current,
    Explicit(Shell),
}

#[derive(Debug, Eq, PartialEq)]
pub enum InstallOutcome {
    Installed {
        shell: Shell,
        profiles: Vec<PathBuf>,
    },
    AlreadyInstalled {
        shell: Shell,
        profiles: Vec<PathBuf>,
    },
}

impl InstallOutcome {
    pub const fn shell(&self) -> Shell {
        match self {
            Self::Installed { shell, .. } | Self::AlreadyInstalled { shell, .. } => *shell,
        }
    }

    pub fn profiles(&self) -> &[PathBuf] {
        match self {
            Self::Installed { profiles, .. } | Self::AlreadyInstalled { profiles, .. } => profiles,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum UninstallOutcome {
    Uninstalled {
        shell: Shell,
        profiles: Vec<PathBuf>,
    },
    AlreadyUninstalled {
        shell: Shell,
    },
}

impl UninstallOutcome {
    pub const fn shell(&self) -> Shell {
        match self {
            Self::Uninstalled { shell, .. } | Self::AlreadyUninstalled { shell, .. } => *shell,
        }
    }

    pub fn profiles(&self) -> &[PathBuf] {
        match self {
            Self::Uninstalled { profiles, .. } => profiles,
            Self::AlreadyUninstalled { .. } => &[],
        }
    }
}

type AdapterError = Box<dyn std::error::Error>;

struct InstallEdit {
    profiles: Vec<PathBuf>,
    changed: bool,
}

enum UninstallEdit {
    Uninstalled { profiles: Vec<PathBuf> },
    AlreadyUninstalled,
}

impl UninstallEdit {
    fn from_profiles(profiles: Vec<PathBuf>) -> Self {
        if profiles.is_empty() {
            Self::AlreadyUninstalled
        } else {
            Self::Uninstalled { profiles }
        }
    }
}

struct Adapter {
    name: &'static str,
    display_name: &'static str,
    activation_command: &'static str,
    matches: fn(&Path) -> bool,
    install: fn(&Selection) -> Result<InstallEdit, AdapterError>,
    uninstall: fn(&Selection) -> Result<UninstallEdit, AdapterError>,
    hook: fn(&[&str]) -> String,
}

static ADAPTERS: &[Adapter] = &[bash::ADAPTER, powershell::ADAPTER];

pub fn shells() -> impl ExactSizeIterator<Item = Shell> + Clone {
    ADAPTERS.iter().map(Shell)
}

pub fn shell(name: &str) -> Option<Shell> {
    ADAPTERS
        .iter()
        .find(|adapter| adapter.name == name)
        .map(Shell)
}

pub fn install(target: ShellTarget) -> Result<InstallOutcome, Error> {
    let resolved = resolve(target)?;
    let edit = (resolved.adapter.install)(&resolved.selection).map_err(Error::adapter)?;

    if edit.changed {
        Ok(InstallOutcome::Installed {
            shell: Shell(resolved.adapter),
            profiles: edit.profiles,
        })
    } else {
        Ok(InstallOutcome::AlreadyInstalled {
            shell: Shell(resolved.adapter),
            profiles: edit.profiles,
        })
    }
}

pub fn uninstall(target: ShellTarget) -> Result<UninstallOutcome, Error> {
    let resolved = resolve(target)?;
    let edit = (resolved.adapter.uninstall)(&resolved.selection).map_err(Error::adapter)?;

    match edit {
        UninstallEdit::Uninstalled { profiles } => Ok(UninstallOutcome::Uninstalled {
            shell: Shell(resolved.adapter),
            profiles,
        }),
        UninstallEdit::AlreadyUninstalled => Ok(UninstallOutcome::AlreadyUninstalled {
            shell: Shell(resolved.adapter),
        }),
    }
}

pub fn hook<'a>(shell: Shell, commands: impl IntoIterator<Item = &'a str>) -> String {
    let commands: Vec<_> = commands.into_iter().collect();
    (shell.0.hook)(&commands)
}

struct ResolvedAdapter {
    adapter: &'static Adapter,
    selection: Selection,
}

enum Selection {
    Detected { executable: PathBuf },
    Explicit,
}

fn resolve(target: ShellTarget) -> Result<ResolvedAdapter, Error> {
    match target {
        ShellTarget::Current => {
            let process = detection::current().map_err(Error::detection)?;
            let adapter = ADAPTERS
                .iter()
                .find(|adapter| (adapter.matches)(process.command()))
                .ok_or_else(Error::unsupported_current_shell)?;
            Ok(ResolvedAdapter {
                adapter,
                selection: Selection::Detected {
                    executable: process.into_executable(),
                },
            })
        }
        ShellTarget::Explicit(shell) => Ok(ResolvedAdapter {
            adapter: shell.0,
            selection: Selection::Explicit,
        }),
    }
}

#[derive(Debug)]
pub struct Error(ErrorKind);

#[derive(Debug)]
enum ErrorKind {
    UnsupportedCurrentShell,
    Detection(detection::Error),
    Adapter(AdapterError),
}

impl Error {
    fn unsupported_current_shell() -> Self {
        Self(ErrorKind::UnsupportedCurrentShell)
    }

    fn detection(error: detection::Error) -> Self {
        Self(ErrorKind::Detection(error))
    }

    fn adapter(source: AdapterError) -> Self {
        Self(ErrorKind::Adapter(source))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            ErrorKind::UnsupportedCurrentShell => {
                formatter.write_str(
                    "could not detect a supported current shell; rerun the command with ",
                )?;
                for (index, adapter) in ADAPTERS.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(if index + 1 == ADAPTERS.len() {
                            " or "
                        } else {
                            ", "
                        })?;
                    }
                    write!(formatter, "`{}`", adapter.name)?;
                }
                Ok(())
            }
            ErrorKind::Detection(error) => error.fmt(formatter),
            ErrorKind::Adapter(source) => source.fmt(formatter),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.0 {
            ErrorKind::UnsupportedCurrentShell => None,
            ErrorKind::Detection(error) => Some(error),
            ErrorKind::Adapter(source) => Some(source.as_ref()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_advertised_shell_has_one_round_trippable_name() {
        let mut names = HashSet::new();

        for registered in shells() {
            assert!(names.insert(registered.name()));
            assert_eq!(shell(registered.name()), Some(registered));
        }
        assert!(!names.is_empty());
    }
}
