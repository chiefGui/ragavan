use super::{Runner, deliver_directly};
use crate::{DevelopmentCommand, package::PackageTarget};
use std::ffi::OsString;

pub(super) const ADAPTER: Runner = Runner {
    command: "bun",
    recognize,
};

fn recognize(arguments: &[OsString]) -> Option<DevelopmentCommand<'_>> {
    let script_arguments = match arguments {
        [script, script_arguments @ ..] if script == "dev" => script_arguments,
        [run, script, script_arguments @ ..] if run == "run" && script == "dev" => script_arguments,
        _ => return None,
    };

    Some(DevelopmentCommand::new(
        "bun dev",
        "dev",
        PackageTarget::WorkingDirectory,
        script_arguments,
        deliver_directly,
    ))
}
