use crate::ResolvedScript;
use ragavan_core::{LaunchPlan, Port};
use std::{ffi::OsString, fmt, path::PathBuf};

pub struct ViteDev;

impl ViteDev {
    pub fn launch_plan(self, port: Port) -> LaunchPlan {
        LaunchPlan::with_additional_arguments(vec![
            "--port".to_owned(),
            port.to_string(),
            "--strictPort".to_owned(),
        ])
    }
}

pub(crate) fn reject_explicit_port(arguments: &[OsString]) -> Result<(), Error> {
    if arguments
        .iter()
        .filter_map(|argument| argument.to_str())
        .any(is_port_argument)
    {
        Err(Error::ExplicitPort)
    } else {
        Ok(())
    }
}

pub(crate) fn recognize(script: ResolvedScript) -> Result<ViteDev, Error> {
    if !is_supported_vite_script(script.command()) {
        return Err(Error::UnsupportedDevScript {
            path: script.package_path().to_owned(),
            script: script.command().to_owned(),
        });
    }
    if script
        .command()
        .split_ascii_whitespace()
        .any(is_port_argument)
    {
        return Err(Error::ExplicitPort);
    }

    Ok(ViteDev)
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
    UnsupportedDevScript { path: PathBuf, script: String },
    ExplicitPort,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedDevScript { path, script } => write!(
                formatter,
                "could not isolate `bun dev`: {} uses unsupported script `{script}`; this slice recognizes Vite",
                path.display()
            ),
            Self::ExplicitPort => formatter.write_str(
                "could not isolate `bun dev`: an explicit `--port` conflicts with Ragavan's worktree port",
            ),
        }
    }
}

impl std::error::Error for Error {}
