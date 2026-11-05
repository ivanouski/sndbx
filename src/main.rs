use std::process::ExitCode;

fn main() -> ExitCode {
    sndbx::exit_code(sndbx::commands::cli())
}
