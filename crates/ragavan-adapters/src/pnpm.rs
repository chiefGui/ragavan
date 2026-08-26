use crate::{DevelopmentCommand, Runner, deliver_directly};
use std::ffi::OsString;

pub(super) const ADAPTER: Runner = Runner {
    command: "pnpm",
    recognize,
};

fn recognize(arguments: &[OsString]) -> Option<DevelopmentCommand<'_>> {
    let (invocation, script_name, script_arguments) = match arguments {
        [run, script, script_arguments @ ..] if run == "run" || run == "run-script" => {
            match script.to_str()? {
                "dev" => ("pnpm dev", "dev", script_arguments),
                "start" => ("pnpm start", "start", script_arguments),
                _ => return None,
            }
        }
        [script, script_arguments @ ..] => match script.to_str()? {
            "dev" => ("pnpm dev", "dev", script_arguments),
            "start" => ("pnpm start", "start", script_arguments),
            _ => return None,
        },
        _ => return None,
    };

    Some(DevelopmentCommand::new(
        invocation,
        script_name,
        script_arguments,
        deliver_directly,
    ))
}
