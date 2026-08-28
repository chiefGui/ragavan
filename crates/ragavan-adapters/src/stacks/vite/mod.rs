mod plus;

use super::{Error as StackError, Stack, StackAdjustment};
use crate::script::Invocation;
use ragavan_core::Port;
use ragavan_diagnostics::{Detail, Diagnostic};
use std::{ffi::OsString, fmt};

pub(super) use plus::ADAPTER as PLUS_ADAPTER;

pub(super) const ADAPTER: Stack = Stack { recognize, adjust };

fn recognize(invocation: &Invocation) -> bool {
    if !invocation.invokes("vite") {
        return false;
    }

    !matches!(
        invocation.arguments().first().map(String::as_str),
        Some("build" | "optimize" | "-h" | "--help" | "-v" | "--version")
    )
}

fn adjust(
    invocation: &Invocation,
    forwarded_arguments: &[OsString],
    runner_invocation: &'static str,
) -> Result<StackAdjustment, StackError> {
    if invocation
        .arguments()
        .iter()
        .any(|argument| is_port_argument(argument))
        || forwarded_arguments
            .iter()
            .filter_map(|argument| argument.to_str())
            .any(is_port_argument)
    {
        return Err(StackError::adapter(Error::ExplicitPort {
            invocation: runner_invocation,
        }));
    }

    Ok(StackAdjustment { port_arguments })
}

fn port_arguments(port: Port) -> Vec<String> {
    vec![
        "--port".to_owned(),
        port.to_string(),
        "--strictPort".to_owned(),
    ]
}

fn is_port_argument(argument: &str) -> bool {
    argument == "--port" || argument.starts_with("--port=")
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
                "could not isolate `{invocation}`: an explicit `--port` conflicts with Ragavan's managed port",
            ),
        }
    }
}

impl std::error::Error for Error {}

impl Diagnostic for Error {
    fn code(&self) -> &'static str {
        match self {
            Self::ExplicitPort { .. } => "stack.vite.port_conflict",
        }
    }

    fn help(&self) -> Option<String> {
        Some("remove the explicit Vite port and let Ragavan provide it".to_owned())
    }

    fn details(&self) -> Vec<Detail> {
        match self {
            Self::ExplicitPort { invocation } => {
                vec![Detail::text("invocation", *invocation)]
            }
        }
    }
}
