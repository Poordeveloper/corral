#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    corrald::run()
}
