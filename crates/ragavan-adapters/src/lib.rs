#![forbid(unsafe_code)]

mod package;
mod runners;
mod script;
mod stacks;

use package::PackageTarget;
use ragavan_core::{LaunchPlan, Port, ServiceScope};
use ragavan_diagnostics::{Detail, Diagnostic};
use script::Script;
use std::{
    ffi::{OsStr, OsString},
    fmt,
    path::{Path, PathBuf},
};

/// Commands for which the shell should install transparent interception.
pub fn commands() -> impl Iterator<Item = &'static str> {
    runners::commands()
}

/// Recognize a registered package runner's development command without inspecting the project.
pub fn development_command<'a>(
    command: &OsStr,
    arguments: &'a [OsString],
) -> Option<DevelopmentCommand<'a>> {
    runners::recognize(command, arguments)
}

/// A package-script development command that may require isolation.
pub struct DevelopmentCommand<'a> {
    invocation: &'static str,
    script_name: &'static str,
    package_target: PackageTarget<'a>,
    forwarded_arguments: &'a [OsString],
    deliver_arguments: fn(Vec<String>) -> Vec<String>,
}

impl<'a> DevelopmentCommand<'a> {
    fn new(
        invocation: &'static str,
        script_name: &'static str,
        package_target: PackageTarget<'a>,
        forwarded_arguments: &'a [OsString],
        deliver_arguments: fn(Vec<String>) -> Vec<String>,
    ) -> Self {
        Self {
            invocation,
            script_name,
            package_target,
            forwarded_arguments,
            deliver_arguments,
        }
    }

    /// Resolve the package service and describe its port-specific adjustment.
    pub fn resolve(
        self,
        working_directory: &Path,
        worktree_root: &Path,
    ) -> Result<IsolationPlan, Error> {
        let package_script = package::find_script(
            working_directory,
            worktree_root,
            self.package_target,
            self.script_name,
        )
        .map_err(|source| {
            Error(ErrorKind::Package {
                invocation: self.invocation,
                source,
            })
        })?;
        let (package_path, service_scope, source) = package_script.into_parts();
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

        let adjustment = stacks::resolve(stacks::ResolvedScript {
            invocation: self.invocation,
            package_path,
            source,
            script,
            arguments: self.forwarded_arguments,
        })
        .map_err(|source| Error(ErrorKind::Stack(source)))?;
        Ok(IsolationPlan {
            service_scope,
            port_arguments: adjustment.port_arguments,
            deliver_arguments: self.deliver_arguments,
        })
    }
}

/// A recognized development command's resolved service and launch adjustment.
pub struct IsolationPlan {
    service_scope: ServiceScope,
    port_arguments: fn(Port) -> Vec<String>,
    deliver_arguments: fn(Vec<String>) -> Vec<String>,
}

impl IsolationPlan {
    /// Return the discovered service's scope within its Git worktree.
    pub fn service_scope(&self) -> &ServiceScope {
        &self.service_scope
    }

    /// Build the arguments that must be appended to the original command.
    pub fn launch_plan(self, port: Port) -> LaunchPlan {
        let arguments = (self.port_arguments)(port);
        LaunchPlan::with_additional_arguments((self.deliver_arguments)(arguments))
    }
}

#[derive(Debug)]
pub struct Error(ErrorKind);

#[derive(Debug)]
enum ErrorKind {
    Package {
        invocation: &'static str,
        source: package::Error,
    },
    UnsupportedSyntax {
        invocation: &'static str,
        path: PathBuf,
        script: String,
        source: script::Error,
    },
    Stack(stacks::Error),
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
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.0 {
            ErrorKind::Package { source, .. } => Some(source),
            ErrorKind::UnsupportedSyntax { source, .. } => Some(source),
            ErrorKind::Stack(error) => Some(error),
        }
    }
}

impl Diagnostic for Error {
    fn code(&self) -> &'static str {
        match &self.0 {
            ErrorKind::Package { source, .. } => source.code(),
            ErrorKind::UnsupportedSyntax { source, .. } => source.code(),
            ErrorKind::Stack(error) => error.code(),
        }
    }

    fn help(&self) -> Option<String> {
        match &self.0 {
            ErrorKind::Package { source, .. } => source.help(),
            ErrorKind::UnsupportedSyntax { source, .. } => source.help(),
            ErrorKind::Stack(error) => error.help(),
        }
    }

    fn details(&self) -> Vec<Detail> {
        let (invocation, path, script, source): (_, Option<&Path>, Option<&str>, &dyn Diagnostic) =
            match &self.0 {
                ErrorKind::Package { invocation, source } => (*invocation, None, None, source),
                ErrorKind::UnsupportedSyntax {
                    invocation,
                    path,
                    script,
                    source,
                } => (*invocation, Some(path), Some(script), source),
                ErrorKind::Stack(error) => return error.details(),
            };
        let mut details = vec![Detail::text("invocation", invocation)];
        if let Some(path) = path {
            details.push(Detail::text("package", path.display().to_string()));
        }
        if let Some(script) = script {
            details.push(Detail::text("script", script));
        }
        details.extend(source.details());
        details
    }
}
