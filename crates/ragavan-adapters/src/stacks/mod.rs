mod next;
mod vite;

use crate::script::{Invocation, Script};
use ragavan_core::Port;
use std::{ffi::OsString, fmt, path::PathBuf};

struct Stack {
    recognize: fn(&Invocation) -> bool,
    adjust: fn(&Invocation, &[OsString], &'static str) -> Result<StackAdjustment, Error>,
}

pub(super) struct StackAdjustment {
    pub(super) port_arguments: fn(Port) -> Vec<String>,
}

const ADAPTERS: &[Stack] = &[next::ADAPTER, vite::ADAPTER, vite::PLUS_ADAPTER];

pub(super) struct ResolvedScript<'a> {
    pub(super) invocation: &'static str,
    pub(super) package_path: PathBuf,
    pub(super) source: String,
    pub(super) script: Script,
    pub(super) arguments: &'a [OsString],
}

pub(super) fn resolve(script: ResolvedScript<'_>) -> Result<StackAdjustment, Error> {
    let mut recognized = None;

    for (index, invocation) in script.script.invocations().iter().enumerate() {
        for stack in ADAPTERS {
            if !(stack.recognize)(invocation) {
                continue;
            }
            if recognized.is_some() {
                return Err(Error::AmbiguousScript {
                    invocation: script.invocation,
                    path: script.package_path,
                    script: script.source,
                });
            }
            recognized = Some((index, invocation, stack));
        }
    }

    let Some((index, invocation, stack)) = recognized else {
        return Err(Error::UnsupportedScript {
            invocation: script.invocation,
            path: script.package_path,
            script: script.source,
        });
    };
    if index != script.script.invocations().len() - 1 {
        return Err(Error::UnsafeArgumentDelivery {
            invocation: script.invocation,
            path: script.package_path,
            script: script.source,
        });
    }

    (stack.adjust)(invocation, script.arguments, script.invocation)
}

#[derive(Debug)]
pub(super) enum Error {
    Adapter(Box<dyn std::error::Error>),
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
    fn adapter(error: impl std::error::Error + 'static) -> Self {
        Self::Adapter(Box::new(error))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Adapter(error) => error.fmt(formatter),
            Self::UnsupportedScript {
                invocation,
                path,
                script,
            } => write!(
                formatter,
                "could not isolate `{invocation}`: {} uses unsupported script {script:?}; no stack adapter recognizes it as a development server",
                path.display()
            ),
            Self::AmbiguousScript {
                invocation,
                path,
                script,
            } => write!(
                formatter,
                "could not isolate `{invocation}`: {} uses ambiguous script {script:?}; it contains more than one recognized development server",
                path.display()
            ),
            Self::UnsafeArgumentDelivery {
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
        match self {
            Self::Adapter(error) => Some(error.as_ref()),
            Self::UnsupportedScript { .. }
            | Self::AmbiguousScript { .. }
            | Self::UnsafeArgumentDelivery { .. } => None,
        }
    }
}
