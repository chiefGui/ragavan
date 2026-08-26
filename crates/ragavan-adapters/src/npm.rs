use crate::{DevelopmentCommand, Runner, deliver_directly};
use std::ffi::OsString;

pub(super) const ADAPTER: Runner = Runner {
    command: "npm",
    recognize,
};

fn recognize(arguments: &[OsString]) -> Option<DevelopmentCommand<'_>> {
    let (invocation, script_name, runner_arguments) = match arguments {
        [run, script, runner_arguments @ ..] if run == "run" || run == "run-script" => {
            match script.to_str()? {
                "dev" => ("npm run dev", "dev", runner_arguments),
                "start" => ("npm start", "start", runner_arguments),
                _ => return None,
            }
        }
        [start, runner_arguments @ ..] if start == "start" => {
            ("npm start", "start", runner_arguments)
        }
        _ => return None,
    };

    let separator = runner_arguments
        .iter()
        .position(|argument| argument == "--");
    let forwarded_arguments = separator.map_or(&[][..], |index| &runner_arguments[index + 1..]);
    let deliver_arguments = if separator.is_some() {
        deliver_directly
    } else {
        deliver_after_separator
    };

    Some(DevelopmentCommand::new(
        invocation,
        script_name,
        forwarded_arguments,
        deliver_arguments,
    ))
}

fn deliver_after_separator(mut arguments: Vec<String>) -> Vec<String> {
    arguments.insert(0, "--".to_owned());
    arguments
}
