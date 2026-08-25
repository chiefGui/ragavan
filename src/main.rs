#![forbid(unsafe_code)]

use std::{
    env, fmt, io,
    process::{Command, ExitCode, ExitStatus, Output},
};

const ENROLLMENT_KEY: &str = "ragavan.enabled";
const GIT_CONFIG_GET_MISSING: i32 = 1;
const GIT_CONFIG_UNSET_MISSING: i32 = 5;
const USAGE: &str = "\
Usage: ragavan <COMMAND>

Commands:
  enable   Enable Ragavan for the current repository
  status   Show whether Ragavan is enabled
  disable  Disable Ragavan for the current repository
  help     Print this help
";

fn main() -> ExitCode {
    let action = match parse_action() {
        Ok(action) => action,
        Err(message) => {
            eprintln!("error: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    match execute(action) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}

enum Action {
    Enable,
    Status,
    Disable,
    Help,
}

fn parse_action() -> Result<Action, String> {
    let mut arguments = env::args_os().skip(1);
    let command = arguments.next();

    let action = match command.as_deref() {
        None => Action::Help,
        Some(command) if command == "enable" => Action::Enable,
        Some(command) if command == "status" => Action::Status,
        Some(command) if command == "disable" => Action::Disable,
        Some(command) if command == "help" || command == "--help" || command == "-h" => {
            Action::Help
        }
        Some(command) => {
            return Err(format!("unknown command `{}`", command.to_string_lossy()));
        }
    };

    if let Some(argument) = arguments.next() {
        return Err(format!(
            "unexpected argument `{}`",
            argument.to_string_lossy()
        ));
    }

    Ok(action)
}

fn execute(action: Action) -> Result<(), Failure> {
    match action {
        Action::Help => print!("{USAGE}"),
        Action::Enable => println!("{}", enable()?),
        Action::Status => println!("{}", enrollment()?),
        Action::Disable => println!("{}", disable()?),
    }

    Ok(())
}

enum Enrollment {
    Enabled,
    Disabled,
}

impl fmt::Display for Enrollment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Enabled => formatter.write_str("Ragavan is enabled for this repository."),
            Self::Disabled => formatter.write_str("Ragavan is disabled for this repository."),
        }
    }
}

fn enable() -> Result<Enrollment, Failure> {
    const OPERATION: &str = "enable Ragavan for the repository";
    let output = git(
        OPERATION,
        &["config", "--local", "--replace-all", ENROLLMENT_KEY, "true"],
    )?;

    if output.status.success() {
        Ok(Enrollment::Enabled)
    } else {
        Err(Failure::git(OPERATION, output))
    }
}

fn enrollment() -> Result<Enrollment, Failure> {
    const OPERATION: &str = "read the repository enrollment";
    let output = git(
        OPERATION,
        &["config", "--local", "--bool", "--get", ENROLLMENT_KEY],
    )?;

    if output.status.success() {
        return match output.stdout.trim_ascii() {
            b"true" => Ok(Enrollment::Enabled),
            b"false" => Ok(Enrollment::Disabled),
            _ => Err(Failure::UnexpectedGitOutput {
                operation: OPERATION,
                output: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            }),
        };
    }
    if output.status.code() == Some(GIT_CONFIG_GET_MISSING) {
        return Ok(Enrollment::Disabled);
    }

    Err(Failure::git(OPERATION, output))
}

fn disable() -> Result<Enrollment, Failure> {
    const OPERATION: &str = "disable Ragavan for the repository";
    let output = git(
        OPERATION,
        &["config", "--local", "--unset-all", ENROLLMENT_KEY],
    )?;

    if output.status.success() || output.status.code() == Some(GIT_CONFIG_UNSET_MISSING) {
        Ok(Enrollment::Disabled)
    } else {
        Err(Failure::git(OPERATION, output))
    }
}

fn git(operation: &'static str, arguments: &[&str]) -> Result<Output, Failure> {
    Command::new("git")
        .args(arguments)
        .output()
        .map_err(|source| Failure::StartGit { operation, source })
}

#[derive(Debug)]
enum Failure {
    StartGit {
        operation: &'static str,
        source: io::Error,
    },
    Git {
        operation: &'static str,
        status: ExitStatus,
        detail: String,
    },
    UnexpectedGitOutput {
        operation: &'static str,
        output: String,
    },
}

impl Failure {
    fn git(operation: &'static str, output: Output) -> Self {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = if stderr.trim().is_empty() {
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        } else {
            stderr.trim().to_owned()
        };

        Self::Git {
            operation,
            status: output.status,
            detail,
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StartGit { operation, source } => {
                write!(formatter, "could not start Git to {operation}: {source}")
            }
            Self::Git {
                operation,
                status,
                detail,
            } => {
                write!(formatter, "could not {operation} ({status})")?;
                if !detail.is_empty() {
                    write!(formatter, ": {detail}")?;
                }
                Ok(())
            }
            Self::UnexpectedGitOutput { operation, output } => write!(
                formatter,
                "could not {operation}: Git returned an unexpected value `{output}`"
            ),
        }
    }
}
