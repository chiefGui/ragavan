#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    ragavan_cli::run(std::env::args_os())
}
