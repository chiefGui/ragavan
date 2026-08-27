mod bun;
mod npm;
mod pnpm;

use crate::DevelopmentCommand;
use std::ffi::{OsStr, OsString};

struct Runner {
    command: &'static str,
    recognize: for<'a> fn(&'a [OsString]) -> Option<DevelopmentCommand<'a>>,
}

const ADAPTERS: &[Runner] = &[bun::ADAPTER, npm::ADAPTER, pnpm::ADAPTER];

pub(super) fn commands() -> impl Iterator<Item = &'static str> {
    ADAPTERS.iter().map(|runner| runner.command)
}

pub(super) fn recognize<'a>(
    command: &OsStr,
    arguments: &'a [OsString],
) -> Option<DevelopmentCommand<'a>> {
    let command = command.to_str()?;
    let runner = ADAPTERS.iter().find(|runner| runner.command == command)?;

    (runner.recognize)(arguments)
}

fn deliver_directly(arguments: Vec<String>) -> Vec<String> {
    arguments
}
