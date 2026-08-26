use crate::{Error as AdapterError, Invocation, Stack, StackAdjustment};
use ragavan_core::Port;
use std::{ffi::OsString, fmt};

pub(super) const ADAPTER: Stack = Stack { recognize, adjust };

fn recognize(invocation: &Invocation) -> bool {
    invocation.invokes("vite")
}

pub(super) fn adjust(
    invocation: &Invocation,
    forwarded_arguments: &[OsString],
    runner_invocation: &'static str,
) -> Result<StackAdjustment, AdapterError> {
    if invocation
        .arguments()
        .iter()
        .any(|argument| is_port_argument(argument))
        || forwarded_arguments
            .iter()
            .filter_map(|argument| argument.to_str())
            .any(is_port_argument)
    {
        return Err(AdapterError::stack(Error::ExplicitPort {
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
