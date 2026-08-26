use crate::{Error as AdapterError, ResolvedScript, StackAdjustment};
use ragavan_core::Port;
use std::{ffi::OsString, fmt};

fn port_arguments(port: Port) -> Vec<String> {
    vec![
        "--port".to_owned(),
        port.to_string(),
        "--strictPort".to_owned(),
    ]
}

fn reject_explicit_port(arguments: &[OsString], invocation: &'static str) -> Result<(), Error> {
    if arguments
        .iter()
        .filter_map(|argument| argument.to_str())
        .any(is_port_argument)
    {
        Err(Error::ExplicitPort { invocation })
    } else {
        Ok(())
    }
}

pub(crate) fn recognize(
    script: &ResolvedScript<'_>,
) -> Result<Option<StackAdjustment>, AdapterError> {
    if !is_supported_vite_script(script.command()) {
        return Ok(None);
    }
    reject_explicit_port(script.arguments(), script.invocation()).map_err(AdapterError::stack)?;
    if script
        .command()
        .split_ascii_whitespace()
        .any(is_port_argument)
    {
        return Err(AdapterError::stack(Error::ExplicitPort {
            invocation: script.invocation(),
        }));
    }

    Ok(Some(StackAdjustment::new(port_arguments)))
}

fn is_port_argument(argument: &str) -> bool {
    argument == "--port" || argument.starts_with("--port=")
}

fn is_supported_vite_script(script: &str) -> bool {
    if script
        .chars()
        .any(|character| "\r\n&|;<>".contains(character))
    {
        return false;
    }

    let Some(command) = script.split_ascii_whitespace().next() else {
        return false;
    };
    let command = command.rsplit(['/', '\\']).next().unwrap_or(command);

    matches!(command, "vite" | "vite.cmd" | "vite.exe")
}

#[derive(Debug)]
pub(crate) enum Error {
    ExplicitPort { invocation: &'static str },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExplicitPort { invocation } => write!(
                formatter,
                "could not isolate `{invocation}`: an explicit `--port` conflicts with Ragavan's worktree port",
            ),
        }
    }
}

impl std::error::Error for Error {}
