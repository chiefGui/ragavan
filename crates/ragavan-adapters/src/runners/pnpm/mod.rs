use super::{Runner, deliver_directly};
use crate::{
    DevelopmentCommand,
    package::{PackageSelector, PackageTarget, SelectorBase},
};
use std::{
    ffi::{OsStr, OsString},
    path::Path,
};

pub(super) const ADAPTER: Runner = Runner {
    command: "pnpm",
    recognize,
};

fn recognize(arguments: &[OsString]) -> Option<DevelopmentCommand<'_>> {
    let mut package_target = PackageTarget::WorkingDirectory;
    let mut recursive = false;
    let arguments = consume_target_options(arguments, &mut package_target, &mut recursive)?;
    let (invocation, script_name, script_arguments) = match arguments {
        [run, arguments @ ..] if run == "run" || run == "run-script" => {
            let arguments = consume_target_options(arguments, &mut package_target, &mut recursive)?;
            let [script, script_arguments @ ..] = arguments else {
                return None;
            };
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
        [] => return None,
    };
    if let PackageTarget::Selected(PackageSelector::Name(selector)) = package_target {
        package_target = if !is_exact_selector(selector) {
            PackageTarget::NonExact(selector)
        } else if is_directory_selector(selector) {
            PackageTarget::Selected(PackageSelector::Directory {
                value: selector,
                relative_to: SelectorBase::WorkingDirectory,
            })
        } else {
            package_target
        };
    }
    if recursive && matches!(package_target, PackageTarget::WorkingDirectory) {
        package_target = PackageTarget::Multiple;
    }

    Some(DevelopmentCommand::new(
        invocation,
        script_name,
        package_target,
        script_arguments,
        deliver_directly,
    ))
}

fn consume_target_options<'a>(
    mut arguments: &'a [OsString],
    package_target: &mut PackageTarget<'a>,
    recursive: &mut bool,
) -> Option<&'a [OsString]> {
    loop {
        match arguments {
            [option, remaining @ ..]
                if option == "-r"
                    || option == "--recursive"
                    || option == "m"
                    || option == "multi"
                    || option == "recursive" =>
            {
                *recursive = true;
                arguments = remaining;
            }
            [option, selector, remaining @ ..] if option == "--filter" || option == "-F" => {
                let option = if option == "--filter" {
                    "--filter"
                } else {
                    "-F"
                };
                package_target.select(PackageSelector::Name(selector), option);
                arguments = remaining;
            }
            [option, remaining @ ..] => {
                let option = option.to_str()?;
                let selection = option
                    .strip_prefix("--filter=")
                    .map(|selector| (selector, "--filter"))
                    .or_else(|| option.strip_prefix("-F=").map(|selector| (selector, "-F")));
                let Some((selector, option)) = selection else {
                    return Some(arguments);
                };
                package_target.select(PackageSelector::Name(OsStr::new(selector)), option);
                arguments = remaining;
            }
            [] => return Some(arguments),
        }
    }
}

fn is_exact_selector(selector: &OsStr) -> bool {
    let Some(selector) = selector.to_str() else {
        return true;
    };

    selector != "."
        && !selector.starts_with('!')
        && !selector.contains("...")
        && !selector
            .chars()
            .any(|character| matches!(character, '*' | '?' | '[' | ']' | '{' | '}' | '^'))
}

fn is_directory_selector(selector: &OsStr) -> bool {
    if Path::new(selector).is_absolute() {
        return true;
    }

    selector.to_str().is_some_and(|selector| {
        selector.starts_with("./")
            || selector.starts_with(".\\")
            || selector.starts_with("../")
            || selector.starts_with("..\\")
            || selector == ".."
    })
}
