#![forbid(unsafe_code)]

mod enrollment;
mod integration;
mod presentation;

use clap::{
    CommandFactory, Parser, Subcommand,
    builder::{PossibleValuesParser, TypedValueParser},
    error::{ContextKind, ContextValue, ErrorKind},
};
use presentation::Format;
use ragavan_core::{LaunchPlan, ServiceIdentity};
use ragavan_diagnostics::{Detail, Diagnostic};
use std::{
    env,
    ffi::OsString,
    io,
    path::Path,
    process::{Command as ProcessCommand, ExitStatus},
};
use thiserror::Error;

/// Run Ragavan's command-line interface and return its process status code.
pub fn run(arguments: impl IntoIterator<Item = OsString>) -> i32 {
    let arguments: Vec<_> = arguments.into_iter().collect();

    match ragavan_shell::protocol::parse(arguments.get(1..).unwrap_or_default()) {
        Ok(Some(request)) => return run_command(request),
        Ok(None) => {}
        Err(error) => return presentation::report(&error, Format::Human, 1),
    }

    let json_requested = arguments.iter().any(|argument| argument == "--json");
    let cli = match Cli::try_parse_from(arguments) {
        Ok(cli) => cli,
        Err(error) => return report_parse_error(error, json_requested),
    };
    let format = if cli.json {
        Format::Json
    } else {
        Format::Human
    };
    let Some(command) = cli.command else {
        if format == Format::Json {
            return report_usage(
                "cli.command.missing",
                "a command is required",
                Some("run `ragavan --help` to list available commands"),
                format,
            );
        }
        return report_output(
            presentation::print(Cli::command().render_help()),
            "command help",
            format,
        );
    };

    match command {
        Command::Install { shell } => complete(
            ragavan_shell::install(shell_target(shell)).map_err(Failure::from),
            format,
        ),
        Command::Uninstall { shell } => complete(
            ragavan_shell::uninstall(shell_target(shell)).map_err(Failure::from),
            format,
        ),
        Command::Enable => complete(ragavan_git::enable().map_err(Failure::from), format),
        Command::Status => complete(ragavan_git::status().map_err(Failure::from), format),
        Command::Disable => complete(ragavan_git::disable().map_err(Failure::from), format),
        Command::Hook { shell } => {
            if format == Format::Json {
                return report_usage(
                    "cli.output.unsupported",
                    "structured output is unavailable for `hook`; its output is the shell integration",
                    Some("run `ragavan hook` without `--json`"),
                    format,
                );
            }
            let native_executable = match env::current_exe() {
                Ok(executable) => executable,
                Err(source) => {
                    return report_failure(Failure::CurrentExecutable(source), format);
                }
            };
            let hook = match ragavan_shell::hook(
                shell,
                &native_executable,
                ragavan_adapters::commands(),
            ) {
                Ok(hook) => hook,
                Err(error) => return report_failure(Failure::from(error), format),
            };
            report_output(presentation::print(hook), "shell integration", format)
        }
    }
}

fn complete(result: Result<impl presentation::Response, Failure>, format: Format) -> i32 {
    match result {
        Ok(response) => report_output(
            presentation::present(&response, format),
            "command result",
            format,
        ),
        Err(error) => report_failure(error, format),
    }
}

#[derive(Parser)]
#[command(
    name = "ragavan",
    bin_name = "ragavan",
    version,
    about = "Zero-configuration isolation for concurrent Git worktrees",
    styles = presentation::CLI_STYLES
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
        #[arg(value_parser = shell_parser())]
        shell: Option<ragavan_shell::Shell>,
    },
    /// Remove persistent integration from the current shell.
    Uninstall {
        /// Select a shell when automatic detection is unavailable.
        #[arg(value_parser = shell_parser())]
        shell: Option<ragavan_shell::Shell>,
    },
    /// Enable Ragavan for the current repository.
    Enable,
    /// Show whether Ragavan is enabled.
    Status,
    /// Disable Ragavan for the current repository.
    Disable,
    /// Print shell integration.
    Hook {
        #[arg(value_parser = shell_parser())]
        shell: ragavan_shell::Shell,
    },
}

fn shell_parser() -> impl TypedValueParser<Value = ragavan_shell::Shell> {
    PossibleValuesParser::new(ragavan_shell::shells().map(|shell| shell.name())).map(|name| {
        ragavan_shell::shell(&name).expect("every advertised shell must remain registered")
    })
}

fn shell_target(shell: Option<ragavan_shell::Shell>) -> ragavan_shell::ShellTarget {
    shell.map_or(ragavan_shell::ShellTarget::Current, |shell| {
        ragavan_shell::ShellTarget::Explicit(shell)
    })
}

fn report_parse_error(error: clap::Error, json_requested: bool) -> i32 {
    if matches!(
        error.kind(),
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
    ) {
        if let Err(source) = error.print() {
            return report_failure(
                Failure::WriteOutput {
                    output: "command help",
                    source,
                },
                Format::Human,
            );
        }
        return 0;
    }

    let diagnostic = UsageError::from_clap(&error);
    presentation::report(
        &diagnostic,
        if json_requested {
            Format::Json
        } else {
            Format::Human
        },
        2,
    )
}

fn report_usage(code: &'static str, message: &str, help: Option<&str>, format: Format) -> i32 {
    presentation::report(
        &UsageError {
            code,
            message: message.to_owned(),
            help: help.map(str::to_owned),
            details: Vec::new(),
        },
        format,
        2,
    )
}

fn report_failure(error: Failure, format: Format) -> i32 {
    presentation::report(&error, format, 1)
}

fn report_output(result: io::Result<()>, output: &'static str, format: Format) -> i32 {
    match result {
        Ok(()) => 0,
        Err(source) => report_failure(Failure::WriteOutput { output, source }, format),
    }
}

#[derive(Debug, Error)]
#[error("{message}")]
struct UsageError {
    code: &'static str,
    message: String,
    help: Option<String>,
    details: Vec<Detail>,
}

impl UsageError {
    fn from_clap(error: &clap::Error) -> Self {
        let message = error.to_string();
        let message = message
            .strip_prefix("error: ")
            .unwrap_or(&message)
            .split("\n\n")
            .next()
            .unwrap_or_default()
            .trim_end()
            .to_owned();
        Self {
            code: clap_error_code(error.kind()),
            message,
            help: clap_error_help(error),
            details: clap_error_details(error),
        }
    }
}

impl Diagnostic for UsageError {
    fn code(&self) -> &'static str {
        self.code
    }

    fn help(&self) -> Option<String> {
        self.help.clone()
    }

    fn details(&self) -> Vec<Detail> {
        self.details.clone()
    }
}

fn clap_error_code(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::InvalidValue | ErrorKind::ValueValidation => "cli.value.invalid",
        ErrorKind::UnknownArgument => "cli.argument.unknown",
        ErrorKind::InvalidSubcommand => "cli.command.invalid",
        ErrorKind::NoEquals => "cli.value.equals_required",
        ErrorKind::TooManyValues => "cli.value.too_many",
        ErrorKind::TooFewValues => "cli.value.too_few",
        ErrorKind::WrongNumberOfValues => "cli.value.count_invalid",
        ErrorKind::ArgumentConflict => "cli.argument.conflict",
        ErrorKind::MissingRequiredArgument => "cli.argument.missing",
        ErrorKind::MissingSubcommand => "cli.command.missing",
        ErrorKind::InvalidUtf8 => "cli.argument.non_unicode",
        ErrorKind::Io => "cli.output.write",
        ErrorKind::Format => "cli.output.format",
        ErrorKind::DisplayHelp
        | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        | ErrorKind::DisplayVersion => "cli.display",
        _ => "cli.usage.invalid",
    }
}

fn clap_error_help(error: &clap::Error) -> Option<String> {
    for kind in [
        ContextKind::SuggestedCommand,
        ContextKind::SuggestedSubcommand,
        ContextKind::SuggestedArg,
        ContextKind::SuggestedValue,
        ContextKind::Suggested,
    ] {
        if let Some(value) = error.get(kind) {
            return Some(format!("try `{value}`"));
        }
    }
    Some("run `ragavan --help` to see available commands and options".to_owned())
}

fn clap_error_details(error: &clap::Error) -> Vec<Detail> {
    error
        .context()
        .filter_map(|(kind, value)| {
            let name = match kind {
                ContextKind::InvalidSubcommand => "invalid_subcommand",
                ContextKind::InvalidArg => "invalid_argument",
                ContextKind::PriorArg => "prior_argument",
                ContextKind::ValidSubcommand => "valid_subcommands",
                ContextKind::ValidValue => "valid_values",
                ContextKind::InvalidValue => "invalid_value",
                ContextKind::ActualNumValues => "actual_value_count",
                ContextKind::ExpectedNumValues => "expected_value_count",
                ContextKind::MinValues => "minimum_value_count",
                ContextKind::SuggestedCommand => "suggested_command",
                ContextKind::SuggestedSubcommand => "suggested_subcommand",
                ContextKind::SuggestedArg => "suggested_argument",
                ContextKind::SuggestedValue => "suggested_value",
                ContextKind::TrailingArg => "trailing_argument",
                ContextKind::Suggested => "suggested",
                ContextKind::Usage | ContextKind::Custom => return None,
                _ => return None,
            };
            Some(match value {
                ContextValue::Strings(values) => Detail::list(name, values),
                ContextValue::StyledStrs(values) => {
                    Detail::list(name, values.iter().map(ToString::to_string))
                }
                ContextValue::Number(value) if *value >= 0 => Detail::number(name, *value as u64),
                ContextValue::None => return None,
                _ => Detail::text(name, value.to_string()),
            })
        })
        .collect()
}

fn run_command(request: ragavan_shell::protocol::RunRequest<'_>) -> i32 {
    match execute_command(&request) {
        Ok(status) => child_exit_code(status),
        Err(error) => presentation::report(&error, Format::Human, 1),
    }
}

fn execute_command(
    request: &ragavan_shell::protocol::RunRequest<'_>,
) -> Result<ExitStatus, Failure> {
    let mut command = ProcessCommand::new(request.program());
    command
        .args(request.launch_arguments())
        .args(request.arguments());
    let Some((lease, plan)) = isolate_command(request.command(), request.arguments())? else {
        return command.status().map_err(|source| Failure::RunCommand {
            program: request.program().to_owned(),
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

#[derive(Debug, Error)]
enum Failure {
    #[error("could not locate the running Ragavan executable: {0}")]
    CurrentExecutable(#[source] io::Error),
    #[error(transparent)]
    Git(#[from] ragavan_git::Error),
    #[error(transparent)]
    Adapter(#[from] ragavan_adapters::Error),
    #[error(transparent)]
    Runtime(#[from] ragavan_runtime::Error),
    #[error(transparent)]
    Shell(#[from] ragavan_shell::Error),
    #[error("could not run {}: {source}", Path::new(.program).display())]
    RunCommand {
        program: OsString,
        #[source]
        source: io::Error,
    },
    #[error("could not write {output}: {source}")]
    WriteOutput {
        output: &'static str,
        #[source]
        source: io::Error,
    },
}

impl Diagnostic for Failure {
    fn code(&self) -> &'static str {
        match self {
            Self::CurrentExecutable(_) => "cli.executable.locate",
            Self::Git(error) => error.code(),
            Self::Adapter(error) => error.code(),
            Self::Runtime(error) => error.code(),
            Self::Shell(error) => error.code(),
            Self::RunCommand { .. } => "runner.process.start",
            Self::WriteOutput { .. } => "cli.output.write",
        }
    }

    fn help(&self) -> Option<String> {
        match self {
            Self::CurrentExecutable(_) => {
                Some("reinstall Ragavan, then retry from a fresh shell".to_owned())
            }
            Self::Git(error) => error.help(),
            Self::Adapter(error) => error.help(),
            Self::Runtime(error) => error.help(),
            Self::Shell(error) => error.help(),
            Self::RunCommand { .. } => None,
            Self::WriteOutput { .. } => None,
        }
    }

    fn details(&self) -> Vec<Detail> {
        match self {
            Self::CurrentExecutable(_) => Vec::new(),
            Self::Git(error) => error.details(),
            Self::Adapter(error) => error.details(),
            Self::Runtime(error) => error.details(),
            Self::Shell(error) => error.details(),
            Self::RunCommand { program, .. } => vec![Detail::text(
                "executable",
                Path::new(program).display().to_string(),
            )],
            Self::WriteOutput { output, .. } => vec![Detail::text("output", *output)],
        }
    }
}
