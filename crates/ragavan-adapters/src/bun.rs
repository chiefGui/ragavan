use crate::{Error as AdapterError, ResolvedScript, Runner, package_json, script::Script};
use std::{
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
};

pub(super) const ADAPTER: Runner = Runner {
    command: "bun",
    resolve,
};

fn forward_script_arguments(arguments: Vec<String>) -> Vec<String> {
    arguments
}

fn resolve<'a>(
    arguments: &'a [OsString],
    worktree_root: &Path,
) -> Result<Option<ResolvedScript<'a>>, AdapterError> {
    let Some(bun_dev) = BunDev::recognize(arguments) else {
        return Ok(None);
    };

    Ok(Some(
        bun_dev
            .resolve(worktree_root)
            .map_err(AdapterError::runner)?,
    ))
}

struct BunDev<'a> {
    arguments: &'a [OsString],
    script_index: usize,
}

impl<'a> BunDev<'a> {
    fn recognize(arguments: &'a [OsString]) -> Option<Self> {
        let script_index = match arguments {
            [script, ..] if script == "dev" => 0,
            [run, script, ..] if run == "run" && script == "dev" => 1,
            _ => return None,
        };

        Some(Self {
            arguments,
            script_index,
        })
    }

    fn script_arguments(&self) -> &'a [OsString] {
        &self.arguments[self.script_index + 1..]
    }

    fn resolve(self, worktree_root: &Path) -> Result<ResolvedScript<'a>, Error> {
        let package_script =
            package_json::find_script(worktree_root, "dev").map_err(Error::Package)?;
        let (package_path, source) = package_script.into_parts();
        let script = match Script::parse(&source) {
            Ok(script) => script,
            Err(error) => {
                return Err(Error::UnsupportedSyntax {
                    path: package_path,
                    script: source,
                    source: error,
                });
            }
        };
        let argument_sink = script
            .invocations()
            .len()
            .checked_sub(1)
            .expect("validated scripts contain an invocation");

        Ok(ResolvedScript {
            invocation: "bun dev",
            package_path,
            source,
            script,
            argument_sink,
            arguments: self.script_arguments(),
            forward_script_arguments,
        })
    }
}

#[derive(Debug)]
enum Error {
    Package(package_json::Error),
    UnsupportedSyntax {
        path: PathBuf,
        script: String,
        source: crate::script::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not isolate `bun dev`: ")?;
        match self {
            Self::Package(source) => source.fmt(formatter),
            Self::UnsupportedSyntax {
                path,
                script,
                source,
            } => write!(
                formatter,
                "{} uses unsupported script {script:?}: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Package(source) => Some(source),
            Self::UnsupportedSyntax { source, .. } => Some(source),
        }
    }
}
