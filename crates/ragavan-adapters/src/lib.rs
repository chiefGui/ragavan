#![forbid(unsafe_code)]

mod bun;
mod npm;
mod package_json;
mod pnpm;
mod script;
mod vite;
mod vite_plus;

use ragavan_core::{LaunchPlan, Port};
use script::{Invocation, Script};
use std::{
    ffi::{OsStr, OsString},
    fmt,
    path::{Path, PathBuf},
};

struct Runner {
    command: &'static str,
    recognize: for<'a> fn(&'a [OsString]) -> Option<DevelopmentCommand<'a>>,
}

struct Stack {
    recognize: fn(&Invocation) -> bool,
    adjust: fn(&Invocation, &[OsString], &'static str) -> Result<StackAdjustment, Error>,
}

const RUNNERS: &[Runner] = &[bun::ADAPTER, npm::ADAPTER, pnpm::ADAPTER];

const STACKS: &[Stack] = &[vite::ADAPTER, vite_plus::ADAPTER];

/// Commands for which the shell should install transparent interception.
pub fn commands() -> impl Iterator<Item = &'static str> {
    RUNNERS.iter().map(|runner| runner.command)
}

/// Recognize a registered package runner's development command without inspecting the project.
pub fn development_command<'a>(
    command: &OsStr,
    arguments: &'a [OsString],
) -> Option<DevelopmentCommand<'a>> {
    let command = command.to_str()?;
    let runner = RUNNERS.iter().find(|runner| runner.command == command)?;

    (runner.recognize)(arguments)
}

/// A package-script command that may require worktree isolation.
pub struct DevelopmentCommand<'a> {
    invocation: &'static str,
    script_name: &'static str,
    forwarded_arguments: &'a [OsString],
    deliver_arguments: fn(Vec<String>) -> Vec<String>,
}

impl<'a> DevelopmentCommand<'a> {
    fn new(
        invocation: &'static str,
        script_name: &'static str,
        forwarded_arguments: &'a [OsString],
        deliver_arguments: fn(Vec<String>) -> Vec<String>,
    ) -> Self {
        Self {
            invocation,
            script_name,
            forwarded_arguments,
            deliver_arguments,
        }
    }

    /// Resolve the package script and describe its port-specific adjustment.
    pub fn resolve(self, worktree_root: &Path) -> Result<PortAdjustment, Error> {
        let package_script =
            package_json::find_script(worktree_root, self.script_name).map_err(|source| {
                Error(ErrorKind::Package {
                    invocation: self.invocation,
                    source,
                })
            })?;
        let (package_path, source) = package_script.into_parts();
        let script = match Script::parse(&source) {
            Ok(script) => script,
            Err(source_error) => {
                return Err(Error(ErrorKind::UnsupportedSyntax {
                    invocation: self.invocation,
                    path: package_path,
                    script: source,
                    source: source_error,
                }));
            }
        };

        resolve_stack(ResolvedScript {
            invocation: self.invocation,
            package_path,
            source,
            script,
            arguments: self.forwarded_arguments,
            deliver_arguments: self.deliver_arguments,
        })
    }
}

fn deliver_directly(arguments: Vec<String>) -> Vec<String> {
    arguments
}

fn resolve_stack(script: ResolvedScript<'_>) -> Result<PortAdjustment, Error> {
    let mut recognized = None;

    for (index, invocation) in script.script.invocations().iter().enumerate() {
        for stack in STACKS {
            if !(stack.recognize)(invocation) {
                continue;
            }
            if recognized.is_some() {
                return Err(Error(ErrorKind::AmbiguousScript {
                    invocation: script.invocation,
                    path: script.package_path,
                    script: script.source,
                }));
            }
            recognized = Some((index, invocation, stack));
        }
    }

    let Some((index, invocation, stack)) = recognized else {
        return Err(Error(ErrorKind::UnsupportedScript {
            invocation: script.invocation,
            path: script.package_path,
            script: script.source,
        }));
    };
    if index != script.script.invocations().len() - 1 {
        return Err(Error(ErrorKind::UnsafeArgumentDelivery {
            invocation: script.invocation,
            path: script.package_path,
            script: script.source,
        }));
    }

    let stack_adjustment = (stack.adjust)(invocation, script.arguments, script.invocation)?;
    Ok(PortAdjustment {
        port_arguments: stack_adjustment.port_arguments,
        deliver_arguments: script.deliver_arguments,
    })
}

/// A recognized command's runner-aware adjustment for an allocated port.
pub struct PortAdjustment {
    port_arguments: fn(Port) -> Vec<String>,
    deliver_arguments: fn(Vec<String>) -> Vec<String>,
}

impl PortAdjustment {
    /// Build the arguments that must be appended to the original command.
    pub fn launch_plan(self, port: Port) -> LaunchPlan {
        let arguments = (self.port_arguments)(port);
        LaunchPlan::with_additional_arguments((self.deliver_arguments)(arguments))
    }
}

struct StackAdjustment {
    port_arguments: fn(Port) -> Vec<String>,
}

struct ResolvedScript<'a> {
    invocation: &'static str,
    package_path: PathBuf,
    source: String,
    script: script::Script,
    arguments: &'a [OsString],
    deliver_arguments: fn(Vec<String>) -> Vec<String>,
}

#[derive(Debug)]
pub struct Error(ErrorKind);

#[derive(Debug)]
enum ErrorKind {
    Package {
        invocation: &'static str,
        source: package_json::Error,
    },
    UnsupportedSyntax {
        invocation: &'static str,
        path: PathBuf,
        script: String,
        source: script::Error,
    },
    Stack(Box<dyn std::error::Error>),
    UnsupportedScript {
        invocation: &'static str,
        path: PathBuf,
        script: String,
    },
    AmbiguousScript {
        invocation: &'static str,
        path: PathBuf,
        script: String,
    },
    UnsafeArgumentDelivery {
        invocation: &'static str,
        path: PathBuf,
        script: String,
    },
}

impl Error {
    fn stack(error: impl std::error::Error + 'static) -> Self {
        Self(ErrorKind::Stack(Box::new(error)))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            ErrorKind::Package { invocation, source } => {
                write!(formatter, "could not isolate `{invocation}`: {source}")
            }
            ErrorKind::UnsupportedSyntax {
                invocation,
                path,
                script,
                source,
            } => write!(
                formatter,
                "could not isolate `{invocation}`: {} uses unsupported script {script:?}: {source}",
                path.display()
            ),
            ErrorKind::Stack(error) => error.fmt(formatter),
            ErrorKind::UnsupportedScript {
                invocation,
                path,
                script,
            } => write!(
                formatter,
                "could not isolate `{invocation}`: {} uses unsupported script {script:?}; no stack adapter recognizes it as a development server",
                path.display()
            ),
            ErrorKind::AmbiguousScript {
                invocation,
                path,
                script,
            } => write!(
                formatter,
                "could not isolate `{invocation}`: {} uses ambiguous script {script:?}; it contains more than one recognized development server",
                path.display()
            ),
            ErrorKind::UnsafeArgumentDelivery {
                invocation,
                path,
                script,
            } => write!(
                formatter,
                "could not isolate `{invocation}`: {} uses unsafe script {script:?}; the development server must be the final command so the runner can deliver Ragavan's port arguments",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.0 {
            ErrorKind::Package { source, .. } => Some(source),
            ErrorKind::UnsupportedSyntax { source, .. } => Some(source),
            ErrorKind::Stack(error) => Some(error.as_ref()),
            ErrorKind::UnsupportedScript { .. }
            | ErrorKind::AmbiguousScript { .. }
            | ErrorKind::UnsafeArgumentDelivery { .. } => None,
        }
    }
}
