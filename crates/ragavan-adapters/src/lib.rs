#![forbid(unsafe_code)]

mod bun;
mod vite;

use ragavan_core::{LaunchPlan, Port};
use std::{
    ffi::{OsStr, OsString},
    fmt,
    path::{Path, PathBuf},
};

struct Runner {
    command: &'static str,
    resolve: for<'a> fn(&'a [OsString], &Path) -> Result<Option<ResolvedScript<'a>>, Error>,
}

struct Stack {
    recognize: for<'a> fn(&ResolvedScript<'a>) -> Result<Option<StackAdjustment>, Error>,
}

const RUNNERS: &[Runner] = &[Runner {
    command: bun::COMMAND,
    resolve: bun::resolve,
}];

const STACKS: &[Stack] = &[Stack {
    recognize: vite::recognize,
}];

/// Commands for which the shell should install transparent interception.
pub fn commands() -> impl Iterator<Item = &'static str> {
    RUNNERS.iter().map(|runner| runner.command)
}

/// Recognize a registered command and describe its port-specific adjustment.
pub fn recognize(
    command: &OsStr,
    arguments: &[OsString],
    worktree_root: &Path,
) -> Result<Option<PortAdjustment>, Error> {
    let Some(command) = command.to_str() else {
        return Ok(None);
    };
    let Some(runner) = RUNNERS.iter().find(|runner| runner.command == command) else {
        return Ok(None);
    };
    let Some(script) = (runner.resolve)(arguments, worktree_root)? else {
        return Ok(None);
    };
    for stack in STACKS {
        if let Some(stack_adjustment) = (stack.recognize)(&script)? {
            return Ok(Some(PortAdjustment {
                port_arguments: stack_adjustment.port_arguments,
                forward_script_arguments: script.forward_script_arguments,
            }));
        }
    }

    Err(Error(ErrorKind::UnsupportedScript {
        invocation: script.invocation,
        path: script.package_path,
        script: script.command,
    }))
}

/// A recognized command's runner-aware adjustment for an allocated port.
pub struct PortAdjustment {
    port_arguments: fn(Port) -> Vec<String>,
    forward_script_arguments: fn(Vec<String>) -> Vec<String>,
}

impl PortAdjustment {
    /// Build the arguments that must be appended to the original command.
    pub fn launch_plan(self, port: Port) -> LaunchPlan {
        let arguments = (self.port_arguments)(port);
        LaunchPlan::with_additional_arguments((self.forward_script_arguments)(arguments))
    }
}

struct StackAdjustment {
    port_arguments: fn(Port) -> Vec<String>,
}

impl StackAdjustment {
    fn new(port_arguments: fn(Port) -> Vec<String>) -> Self {
        Self { port_arguments }
    }
}

struct ResolvedScript<'a> {
    invocation: &'static str,
    package_path: PathBuf,
    command: String,
    arguments: &'a [OsString],
    forward_script_arguments: fn(Vec<String>) -> Vec<String>,
}

impl ResolvedScript<'_> {
    fn invocation(&self) -> &'static str {
        self.invocation
    }

    fn command(&self) -> &str {
        &self.command
    }

    fn arguments(&self) -> &[OsString] {
        self.arguments
    }
}

#[derive(Debug)]
pub struct Error(ErrorKind);

#[derive(Debug)]
enum ErrorKind {
    Runner(Box<dyn std::error::Error>),
    Stack(Box<dyn std::error::Error>),
    UnsupportedScript {
        invocation: &'static str,
        path: PathBuf,
        script: String,
    },
}

impl Error {
    fn runner(error: impl std::error::Error + 'static) -> Self {
        Self(ErrorKind::Runner(Box::new(error)))
    }

    fn stack(error: impl std::error::Error + 'static) -> Self {
        Self(ErrorKind::Stack(Box::new(error)))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            ErrorKind::Runner(error) | ErrorKind::Stack(error) => error.fmt(formatter),
            ErrorKind::UnsupportedScript {
                invocation,
                path,
                script,
            } => write!(
                formatter,
                "could not isolate `{invocation}`: {} uses unsupported script `{script}`; no stack adapter recognizes it",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.0 {
            ErrorKind::Runner(error) | ErrorKind::Stack(error) => Some(error.as_ref()),
            ErrorKind::UnsupportedScript { .. } => None,
        }
    }
}
