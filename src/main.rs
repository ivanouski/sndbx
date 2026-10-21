mod allowlist;
mod builder;
mod commands;
mod docker;
mod paths;
mod proxy;
mod sandbox;
mod settings;

use std::fmt;
use std::io::Write;
use std::process::ExitCode;

pub use anyhow::Result;

/// Stops sndbx with a message that isn't an error, printed as is
#[derive(Debug)]
pub struct Abort(pub String);

impl fmt::Display for Abort {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Abort {}

/// Prints a line to stdout, ignoring a closed pipe like `head` leaves behind
pub fn print_line(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
}

fn main() -> ExitCode {
    let Err(e) = commands::cli() else {
        return ExitCode::SUCCESS;
    };

    match e.downcast_ref::<Abort>() {
        Some(Abort(message)) => eprintln!("{message}"),
        // With its causes, like "can't read <path>: <reason>"
        None => eprintln!("Error: {e:#}."),
    }
    ExitCode::FAILURE
}
