#![forbid(unsafe_code)]

fn main() {
    std::process::exit(ragavan_cli::run(std::env::args_os()));
}
