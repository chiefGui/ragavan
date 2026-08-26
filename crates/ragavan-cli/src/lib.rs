#![forbid(unsafe_code)]

use clap::{CommandFactory, Parser, Subcommand, ValueEnum, error::ErrorKind};
use ragavan_core::{Enrollment, LaunchPlan, ServiceIdentity};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    fmt, io,
    process::{Command as ProcessCommand, ExitStatus},
};

const JSON_SCHEMA_VERSION: u8 = 1;

/// Run Ragavan's command-line interface and return its process status code.
pub fn run(arguments: impl IntoIterator<Item = OsString>) -> i32 {
    let arguments: Vec<_> = arguments.into_iter().collect();

    match ragavan_shell::protocol::parse(arguments.get(1..).unwrap_or_default()) {
        Ok(Some(request)) => return run_command(request),
        Ok(None) => {}
        Err(message) => {
            eprintln!("error: {message}");
            return 1;
        }
    }

    let json_requested = arguments.iter().any(|argument| argument == "--json");
    let cli = match Cli::try_parse_from(arguments) {
        Ok(cli) => cli,
        Err(error) => return report_parse_error(error, json_requested),
    };
    let format = if cli.json {
        OutputFormat::Json
    } else {
        OutputFormat::Human
    };
    let Some(command) = cli.command else {
        if format == OutputFormat::Json {
            return report_usage("missing command", format);
        }
        print!("{}", Cli::command().render_help());
        return 0;
    };

    let outcome = match command {
        Command::Install { shell } => ragavan_shell::install(Shell::target(shell))
            .map(Outcome::Installation)
            .map_err(Failure::from),
        Command::Uninstall { shell } => ragavan_shell::uninstall(Shell::target(shell))
            .map(Outcome::Uninstallation)
            .map_err(Failure::from),
        Command::Enable => ragavan_git::enable()
            .map(Outcome::Enrollment)
            .map_err(Failure::from),
        Command::Status => ragavan_git::status()
            .map(Outcome::Enrollment)
            .map_err(Failure::from),
        Command::Disable => ragavan_git::disable()
            .map(Outcome::Enrollment)
            .map_err(Failure::from),
        Command::Hook {
            shell: Shell::PowerShell,
        } => {
            if format == OutputFormat::Json {
                return report_usage(
                    "structured output is unavailable for `hook`; its output is the shell integration",
                    format,
                );
            }
            print!(
                "{}",
                ragavan_shell::powershell_hook(ragavan_adapters::commands())
            );
            return 0;
        }
    };

    match outcome {
        Ok(outcome) => {
            outcome.print(format);
            0
        }
        Err(error) => report_failure(error, format),
    }
}

#[derive(Parser)]
#[command(
    name = "ragavan",
    bin_name = "ragavan",
    version,
    about = "Zero-configuration isolation for concurrent Git worktrees"
)]
struct Cli {
    /// Print command results and errors as JSON.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Install persistent integration for the current shell.
    Install {
        /// Select a shell when automatic detection is unavailable.
        #[arg(value_enum)]
        shell: Option<Shell>,
    },
    /// Remove persistent integration from the current shell.
    Uninstall {
        /// Select a shell when automatic detection is unavailable.
        #[arg(value_enum)]
        shell: Option<Shell>,
    },
    /// Enable Ragavan for the current repository.
    Enable,
    /// Show whether Ragavan is enabled.
    Status,
    /// Disable Ragavan for the current repository.
    Disable,
    /// Print shell integration.
    Hook {
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Shell {
    #[value(name = "powershell")]
    PowerShell,
}

impl Shell {
    fn target(shell: Option<Self>) -> ragavan_shell::ShellTarget {
        match shell {
            None => ragavan_shell::ShellTarget::Current,
            Some(Self::PowerShell) => ragavan_shell::ShellTarget::PowerShell,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum OutputFormat {
    Human,
    Json,
}

enum Outcome {
    Installation(ragavan_shell::InstallOutcome),
    Uninstallation(ragavan_shell::UninstallOutcome),
    Enrollment(Enrollment),
}

impl Outcome {
    fn print(self, format: OutputFormat) {
        match format {
            OutputFormat::Human => self.print_human(),
            OutputFormat::Json => println!("{}", self.json()),
        }
    }

    fn print_human(self) {
        match self {
            Self::Installation(outcome) => print_installation(outcome),
            Self::Uninstallation(outcome) => print_uninstallation(outcome),
            Self::Enrollment(enrollment) => println!("{}", enrollment_message(enrollment)),
        }
    }

    fn json(self) -> Value {
        match self {
            Self::Installation(outcome) => json!({
                "schema_version": JSON_SCHEMA_VERSION,
                "integration": {
                    "shell": "powershell",
                    "state": match outcome {
                        ragavan_shell::InstallOutcome::Installed { .. } => "installed",
                        ragavan_shell::InstallOutcome::AlreadyInstalled { .. } => "already_installed",
                    },
                    "profile": outcome.profile().to_string_lossy(),
                },
            }),
            Self::Uninstallation(outcome) => json!({
                "schema_version": JSON_SCHEMA_VERSION,
                "integration": {
                    "shell": "powershell",
                    "state": match outcome {
                        ragavan_shell::UninstallOutcome::Uninstalled { .. } => "uninstalled",
                        ragavan_shell::UninstallOutcome::AlreadyUninstalled { .. } => "already_uninstalled",
                    },
                    "profile": outcome.profile().to_string_lossy(),
                },
            }),
            Self::Enrollment(enrollment) => json!({
                "schema_version": JSON_SCHEMA_VERSION,
                "enrollment": match enrollment {
                    Enrollment::Enabled => "enabled",
                    Enrollment::Disabled => "disabled",
                },
            }),
        }
    }
}

fn report_parse_error(error: clap::Error, json_requested: bool) -> i32 {
    if matches!(
        error.kind(),
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
    ) {
        if let Err(source) = error.print() {
            eprintln!("error: could not print command help: {source}");
            return 1;
        }
        return 0;
    }

    if json_requested {
        print_json_error("usage", error.to_string().trim_end());
    } else if let Err(source) = error.print() {
        eprintln!("error: could not print command error: {source}");
        return 1;
    }
    2
}

fn report_usage(message: &str, format: OutputFormat) -> i32 {
    match format {
        OutputFormat::Human => eprintln!("error: {message}"),
        OutputFormat::Json => print_json_error("usage", message),
    }
    2
}

fn report_failure(error: Failure, format: OutputFormat) -> i32 {
    match format {
        OutputFormat::Human => eprintln!("error: {error}"),
        OutputFormat::Json => print_json_error("operation", &error.to_string()),
    }
    1
}

fn print_json_error(kind: &str, message: &str) {
    eprintln!(
        "{}",
        json!({
            "schema_version": JSON_SCHEMA_VERSION,
            "error": {
                "kind": kind,
                "message": message,
            },
        })
    );
}

fn print_installation(outcome: ragavan_shell::InstallOutcome) {
    match &outcome {
        ragavan_shell::InstallOutcome::Installed { .. } => {
            println!("Ragavan is installed for PowerShell.");
        }
        ragavan_shell::InstallOutcome::AlreadyInstalled { .. } => {
            println!("Ragavan is already installed for PowerShell.");
        }
    }
    println!("Profile: {}", outcome.profile().display());
    println!("New PowerShell sessions will load Ragavan automatically.");
    println!(
        "To activate this session now, run `Invoke-Expression (ragavan hook powershell | Out-String)`."
    );
}

fn print_uninstallation(outcome: ragavan_shell::UninstallOutcome) {
    match &outcome {
        ragavan_shell::UninstallOutcome::Uninstalled { .. } => {
            println!("Ragavan is uninstalled from PowerShell.");
        }
        ragavan_shell::UninstallOutcome::AlreadyUninstalled { .. } => {
            println!("Ragavan is already uninstalled from PowerShell.");
        }
    }
    println!("Profile: {}", outcome.profile().display());
    println!("New PowerShell sessions will no longer load Ragavan.");
    println!("This PowerShell session remains active until it is closed.");
}

fn enrollment_message(enrollment: Enrollment) -> &'static str {
    match enrollment {
        Enrollment::Enabled => "Ragavan is enabled for this repository.",
        Enrollment::Disabled => "Ragavan is disabled for this repository.",
    }
}

fn run_command(request: ragavan_shell::protocol::RunRequest<'_>) -> i32 {
    match execute_command(&request) {
        Ok(status) => child_exit_code(status),
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    }
}

fn execute_command(
    request: &ragavan_shell::protocol::RunRequest<'_>,
) -> Result<ExitStatus, Failure> {
    let mut command = ProcessCommand::new(request.executable());
    command.args(request.arguments());
    let Some((lease, plan)) = isolate_command(request.command(), request.arguments())? else {
        return command.status().map_err(|source| Failure::RunCommand {
            executable: request.executable().to_owned(),
            source,
        });
    };

    command.args(plan.into_additional_arguments());
    lease.run(&mut command).map_err(Failure::from)
}

fn isolate_command(
    command: &std::ffi::OsStr,
    arguments: &[OsString],
) -> Result<Option<(ragavan_runtime::PortLease, LaunchPlan)>, Failure> {
    let Some(development_command) = ragavan_adapters::development_command(command, arguments)
    else {
        return Ok(None);
    };
    let Some(worktree) = ragavan_git::enrolled_worktree()? else {
        return Ok(None);
    };
    let isolation = development_command.resolve(worktree.root())?;
    let identity = ServiceIdentity::new(worktree.identity()?, isolation.service_scope().clone());
    let lease = ragavan_runtime::acquire_port(&identity)?;
    let plan = isolation.launch_plan(lease.port());

    Ok(Some((lease, plan)))
}

fn child_exit_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }

    1
}

#[derive(Debug)]
enum Failure {
    Git(ragavan_git::Error),
    Adapter(ragavan_adapters::Error),
    Runtime(ragavan_runtime::Error),
    Shell(ragavan_shell::Error),
    RunCommand {
        executable: OsString,
        source: io::Error,
    },
}

impl From<ragavan_git::Error> for Failure {
    fn from(error: ragavan_git::Error) -> Self {
        Self::Git(error)
    }
}

impl From<ragavan_adapters::Error> for Failure {
    fn from(error: ragavan_adapters::Error) -> Self {
        Self::Adapter(error)
    }
}

impl From<ragavan_runtime::Error> for Failure {
    fn from(error: ragavan_runtime::Error) -> Self {
        Self::Runtime(error)
    }
}

impl From<ragavan_shell::Error> for Failure {
    fn from(error: ragavan_shell::Error) -> Self {
        Self::Shell(error)
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(error) => error.fmt(formatter),
            Self::Adapter(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
            Self::Shell(error) => error.fmt(formatter),
            Self::RunCommand { executable, source } => write!(
                formatter,
                "could not run {}: {source}",
                std::path::Path::new(executable).display()
            ),
        }
    }
}

impl std::error::Error for Failure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Git(error) => Some(error),
            Self::Adapter(error) => Some(error),
            Self::Runtime(error) => Some(error),
            Self::Shell(error) => Some(error),
            Self::RunCommand { source, .. } => Some(source),
        }
    }
}
