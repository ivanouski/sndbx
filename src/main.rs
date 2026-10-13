mod builder;
mod commands;
mod docker;
mod options;
mod paths;
mod sandbox;
mod settings;

use std::io::Write;
use std::process::ExitCode;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

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

/// Prints help or a list and exits successfully
pub fn exit_with(text: &str) -> ! {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
    std::process::exit(0)
}

fn main() -> ExitCode {
    let args = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();

    match commands::cli(args) {
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
