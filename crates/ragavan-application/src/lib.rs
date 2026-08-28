#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Front-end-independent Ragavan workflows and read models.

mod dashboard;

pub use dashboard::{
    Dashboard, DashboardRepository, DashboardScope, DashboardService, DashboardWorktree,
    RepositoryState, WorktreeState,
};
pub use ragavan_core::{Enrollment, LeaseState, RepositoryId};

use ragavan_core::{LaunchPlan, ServiceIdentity};
use ragavan_diagnostics::{Detail, Diagnostic};
use std::{
    ffi::{OsStr, OsString},
    fmt, io,
    path::Path,
    process::{Command, ExitStatus},
};
use thiserror::Error as ThisError;

/// Enable Ragavan for the repository containing `directory`.
pub fn enable_repository(directory: &Path) -> Result<Enrollment, Error> {
    let repository = ragavan_git::begin_enable(directory)?;
    ragavan_runtime::register_repository(
        repository.repository_id(),
        repository.common_directory(),
    )?;
    repository.complete().map_err(Error::from)
}

/// Read the enrollment of the repository containing `directory`.
pub fn repository_status(directory: &Path) -> Result<Enrollment, Error> {
    ragavan_git::status(directory).map_err(Error::from)
}

/// Disable Ragavan for the repository containing `directory`.
pub fn disable_repository(directory: &Path) -> Result<Enrollment, Error> {
    let repository = ragavan_git::begin_disable(directory)?;
    ragavan_runtime::unregister_repository(
        repository.repository_id(),
        repository.common_directory(),
    )?;
    repository.complete().map_err(Error::from)
}

/// Read a consistent, one-shot dashboard for the requested scope.
pub fn dashboard(scope: DashboardScope<'_>) -> Result<Dashboard, Error> {
    dashboard::load(scope)
}

/// Render the shell hook for every command Ragavan can isolate.
pub fn shell_hook(shell: ragavan_shell::Shell, native_executable: &Path) -> Result<String, Error> {
    ragavan_shell::hook(shell, native_executable, ragavan_adapters::commands()).map_err(Error::from)
}

/// Run one command intercepted by Ragavan's shell protocol.
///
/// Recognized development commands in enabled repositories acquire their managed
/// port before the child starts and retain the lease until it exits. Every other
/// command runs unchanged.
pub fn run_intercepted_command(
    working_directory: &Path,
    program: &OsStr,
    launch_arguments: &[OsString],
    command: &OsStr,
    arguments: &[OsString],
) -> Result<ExitStatus, Error> {
    let mut process = Command::new(program);
    process
        .current_dir(working_directory)
        .args(launch_arguments)
        .args(arguments);
    let Some((lease, plan)) = isolate_command(working_directory, command, arguments)? else {
        return process.status().map_err(|source| {
            Error(ErrorKind::RunCommand {
                program: program.to_owned(),
                source,
            })
        });
    };

    process.args(plan.into_additional_arguments());
    lease.run(&mut process).map_err(Error::from)
}

fn isolate_command(
    working_directory: &Path,
    command: &OsStr,
    arguments: &[OsString],
) -> Result<Option<(ragavan_runtime::PortLease, LaunchPlan)>, Error> {
    let Some(development_command) = ragavan_adapters::development_command(command, arguments)
    else {
        return Ok(None);
    };
    let Some(worktree) = ragavan_git::managed_worktree(working_directory)? else {
        return Ok(None);
    };
    ragavan_runtime::register_repository(
        worktree.identity().repository_id(),
        worktree.common_directory(),
    )?;
    let isolation = development_command.resolve(working_directory, worktree.root())?;
    let identity = ServiceIdentity::new(
        worktree.identity().clone(),
        isolation.service_scope().clone(),
    );
    let lease = ragavan_runtime::acquire_port(&identity)?;
    let plan = isolation.launch_plan(lease.port());

    Ok(Some((lease, plan)))
}

#[derive(Debug)]
/// A failure produced while carrying out a Ragavan workflow.
pub struct Error(ErrorKind);

impl Error {
    pub(crate) fn dashboard_repository_required() -> Self {
        Self(ErrorKind::DashboardRepositoryRequired)
    }
}

#[derive(Debug, ThisError)]
enum ErrorKind {
    #[error(transparent)]
    Git(#[from] ragavan_git::Error),
    #[error(transparent)]
    Adapter(#[from] ragavan_adapters::Error),
    #[error(transparent)]
    Runtime(#[from] ragavan_runtime::Error),
    #[error(transparent)]
    Shell(#[from] ragavan_shell::Error),
    #[error("the selected directory is not inside a Git worktree")]
    DashboardRepositoryRequired,
    #[error("could not run {}: {source}", Path::new(.program).display())]
    RunCommand {
        program: OsString,
        #[source]
        source: io::Error,
    },
}

impl From<ragavan_git::Error> for Error {
    fn from(error: ragavan_git::Error) -> Self {
        Self(ErrorKind::Git(error))
    }
}

impl From<ragavan_adapters::Error> for Error {
    fn from(error: ragavan_adapters::Error) -> Self {
        Self(ErrorKind::Adapter(error))
    }
}

impl From<ragavan_runtime::Error> for Error {
    fn from(error: ragavan_runtime::Error) -> Self {
        Self(ErrorKind::Runtime(error))
    }
}

impl From<ragavan_shell::Error> for Error {
    fn from(error: ragavan_shell::Error) -> Self {
        Self(ErrorKind::Shell(error))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        std::error::Error::source(&self.0)
    }
}

impl Diagnostic for Error {
    fn code(&self) -> &'static str {
        match &self.0 {
            ErrorKind::Git(error) => error.code(),
            ErrorKind::Adapter(error) => error.code(),
            ErrorKind::Runtime(error) => error.code(),
            ErrorKind::Shell(error) => error.code(),
            ErrorKind::DashboardRepositoryRequired => "dashboard.repository.required",
            ErrorKind::RunCommand { .. } => "runner.process.start",
        }
    }

    fn help(&self) -> Option<String> {
        match &self.0 {
            ErrorKind::Git(error) => error.help(),
            ErrorKind::Adapter(error) => error.help(),
            ErrorKind::Runtime(error) => error.help(),
            ErrorKind::Shell(error) => error.help(),
            ErrorKind::DashboardRepositoryRequired => {
                Some("select a directory inside a Git worktree".to_owned())
            }
            ErrorKind::RunCommand { .. } => None,
        }
    }

    fn details(&self) -> Vec<Detail> {
        match &self.0 {
            ErrorKind::Git(error) => error.details(),
            ErrorKind::Adapter(error) => error.details(),
            ErrorKind::Runtime(error) => error.details(),
            ErrorKind::Shell(error) => error.details(),
            ErrorKind::DashboardRepositoryRequired => Vec::new(),
            ErrorKind::RunCommand { program, .. } => vec![Detail::text(
                "executable",
                Path::new(program).display().to_string(),
            )],
        }
    }
}
