use super::{Error as StackError, Stack, StackAdjustment};
use crate::script::Invocation;
use ragavan_core::Port;
use ragavan_diagnostics::{Detail, Diagnostic};
use std::{ffi::OsString, fmt, path::Path};

pub(super) const ADAPTER: Stack = Stack { recognize, adjust };

fn recognize(invocation: &Invocation) -> bool {
    if !invocation.invokes("next") {
        return false;
    }

    match invocation.arguments().first().map(String::as_str) {
        None | Some("dev" | "start") => true,
        Some("-h" | "--help" | "-v" | "--version") => false,
        Some(argument) => argument.starts_with('-') || is_explicit_path(argument),
    }
}

fn is_explicit_path(argument: &str) -> bool {
    Path::new(argument).has_root()
        || argument == "."
        || argument == ".."
        || argument.contains('/')
        || argument.contains('\\')
}

fn adjust(
    invocation: &Invocation,
    forwarded_arguments: &[OsString],
    runner_invocation: &'static str,
) -> Result<StackAdjustment, StackError> {
    let has_port_argument = invocation
        .arguments()
        .iter()
        .map(String::as_str)
        .chain(
            forwarded_arguments
                .iter()
                .filter_map(|argument| argument.to_str()),
        )
        .any(is_port_argument);
    if invocation.defines_environment("PORT") || has_port_argument {
        return Err(StackError::adapter(Error::ExplicitPort {
            invocation: runner_invocation,
        }));
    }

    Ok(StackAdjustment { port_arguments })
}

fn port_arguments(port: Port) -> Vec<String> {
    vec!["--port".to_owned(), port.to_string()]
}

fn is_port_argument(argument: &str) -> bool {
    argument == "--port" || argument.starts_with("--port=") || argument.starts_with("-p")
}

#[derive(Debug)]
enum Error {
    ExplicitPort { invocation: &'static str },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExplicitPort { invocation } => write!(
                formatter,
                "could not isolate `{invocation}`: an explicit Next.js port conflicts with Ragavan's managed port",
            ),
        }
    }
}

impl std::error::Error for Error {}

impl Diagnostic for Error {
    fn code(&self) -> &'static str {
        match self {
            Self::ExplicitPort { .. } => "stack.next.port_conflict",
        }
    }

    fn help(&self) -> Option<String> {
        Some("remove the explicit Next.js port and let Ragavan provide it".to_owned())
    }

    fn details(&self) -> Vec<Detail> {
        match self {
            Self::ExplicitPort { invocation } => {
                vec![Detail::text("invocation", *invocation)]
            }
        }
    }
}
