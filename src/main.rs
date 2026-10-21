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

/// Prints what sndbx is doing, marked off from the output of the docker commands it runs
pub fn print_status(message: &str) {
    print_line(&format!("==> {message}"));
}

/// The items without the repeated ones, in the order they first appear
pub fn unique<T: PartialEq>(items: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut unique = Vec::new();
    for item in items {
        if !unique.contains(&item) {
            unique.push(item);
        }
    }
    unique
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_keeps_the_first_of_each() {
        assert_eq!(unique(["b", "a", "b", "c", "a"]), ["b", "a", "c"]);
        assert_eq!(unique(Vec::<&str>::new()), Vec::<&str>::new());
    }
}
