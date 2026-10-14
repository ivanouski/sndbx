mod builder;
mod commands;
mod docker;
mod paths;
mod sandbox;
mod settings;

use std::io::Write;
use std::process::ExitCode;

#[derive(Debug)]
pub enum Error {
    /// Printed as "Error: <message>."
    Message(String),
    /// Printed as is
    Abort(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn error<T>(message: impl Into<String>) -> Result<T> {
    Err(Error::Message(message.into()))
}

/// Prints a line to stdout, ignoring a closed pipe like `head` leaves behind
pub fn puts(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
}

fn main() -> ExitCode {
    match commands::cli() {
        Ok(()) => ExitCode::SUCCESS,
        Err(Error::Message(message)) => {
            eprintln!("Error: {message}.");
            ExitCode::FAILURE
        }
        Err(Error::Abort(message)) => {
            if message.ends_with('\n') {
                eprint!("{message}");
            } else {
                eprintln!("{message}");
            }
            ExitCode::FAILURE
        }
    }
}
