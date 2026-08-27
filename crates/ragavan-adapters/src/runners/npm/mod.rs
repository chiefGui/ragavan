use super::{Runner, deliver_directly};
use crate::{
    DevelopmentCommand,
    package::{PackageSelector, PackageTarget, SelectorBase},
};
use std::ffi::{OsStr, OsString};

pub(super) const ADAPTER: Runner = Runner {
    command: "npm",
    recognize,
};

fn recognize(arguments: &[OsString]) -> Option<DevelopmentCommand<'_>> {
    let separator = arguments.iter().position(|argument| argument == "--");
    let runner_arguments = separator.map_or(arguments, |index| &arguments[..index]);
    let forwarded_arguments = separator.map_or(&[][..], |index| &arguments[index + 1..]);
    let mut package_target = PackageTarget::CurrentDirectory;
    let mut include_workspace_root = false;
    let mut words = [None; 3];
    let mut word_count = 0;
    let mut index = 0;

    while index < runner_arguments.len() {
        let argument = &runner_arguments[index];
        if argument == "--workspace" || argument == "-w" {
            let option = if argument == "--workspace" {
                "--workspace"
            } else {
                "-w"
            };
            let Some(selector) = runner_arguments.get(index + 1) else {
                package_target = PackageTarget::MissingValue(option);
                index += 1;
                continue;
            };
            package_target.select(
                PackageSelector::NameOrDirectory {
                    value: selector,
                    relative_to: SelectorBase::WorktreeRoot,
                },
                option,
            );
            index += 2;
            continue;
        }

        let argument = argument.to_str()?;
        if let Some(selector) = argument.strip_prefix("--workspace=") {
            package_target.select(
                PackageSelector::NameOrDirectory {
                    value: OsStr::new(selector),
                    relative_to: SelectorBase::WorktreeRoot,
                },
                "--workspace",
            );
        } else if let Some(selector) = argument.strip_prefix("-w=") {
            package_target.select(
                PackageSelector::NameOrDirectory {
                    value: OsStr::new(selector),
                    relative_to: SelectorBase::WorktreeRoot,
                },
                "-w",
            );
        } else if argument == "--workspaces" || argument == "--ws" {
            package_target = PackageTarget::Multiple;
        } else if let Some(enabled) = argument
            .strip_prefix("--workspaces=")
            .or_else(|| argument.strip_prefix("--ws="))
        {
            if enabled != "false" {
                package_target = PackageTarget::Multiple;
            }
        } else if argument == "--include-workspace-root" || argument == "--iwr" {
            include_workspace_root = true;
        } else if let Some(enabled) = argument
            .strip_prefix("--include-workspace-root=")
            .or_else(|| argument.strip_prefix("--iwr="))
        {
            include_workspace_root = enabled != "false";
        } else if argument == "--no-include-workspace-root" || argument == "--no-iwr" {
            include_workspace_root = false;
        } else {
            if word_count == words.len() {
                return None;
            }
            words[word_count] = Some(OsStr::new(argument));
            word_count += 1;
        }
        index += 1;
    }

    if include_workspace_root && !matches!(package_target, PackageTarget::CurrentDirectory) {
        package_target = PackageTarget::Multiple;
    }

    let (invocation, script_name) = match words {
        [Some(run), Some(script), None] if run == "run" || run == "run-script" => {
            match script.to_str()? {
                "dev" => ("npm run dev", "dev"),
                "start" => ("npm start", "start"),
                _ => return None,
            }
        }
        [Some(start), None, None] if start == "start" => ("npm start", "start"),
        _ => return None,
    };

    let deliver_arguments = if separator.is_some() {
        deliver_directly
    } else {
        deliver_after_separator
    };

    Some(DevelopmentCommand::new(
        invocation,
        script_name,
        package_target,
        forwarded_arguments,
        deliver_arguments,
    ))
}

fn deliver_after_separator(mut arguments: Vec<String>) -> Vec<String> {
    arguments.insert(0, "--".to_owned());
    arguments
}
