//! sndbx's commands, and what `sndbx top` reads and does through them

pub mod allowlist;
pub mod builder;
pub mod commands;
pub mod docker;
pub mod paths;
pub mod proxy;
pub mod removal;
pub mod sandbox;
pub mod settings;

use std::cell::RefCell;
use std::fmt;
use std::io::Write;
use std::process::ExitCode;

pub use anyhow::Result;

thread_local! {
    /// What the thread would have printed, while it works in the background
    static KEPT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// For a thread working in the background of a terminal app, which owns the screen: the docker commands it starts
/// get their own process group, so Ctrl-C meant for a command in the foreground doesn't stop them, and what they
/// print, and sndbx's warnings about running them, are kept for `take_kept_output` instead. sndbx's status lines,
/// like the builder's and `removal`'s, aren't, so such a thread must not call what prints them.
pub fn work_in_background() {
    KEPT.with_borrow_mut(|kept| *kept = Some(String::new()));
}

pub(crate) fn in_background() -> bool {
    KEPT.with_borrow(Option::is_some)
}

/// What the thread kept since the last call
pub fn take_kept_output() -> String {
    KEPT.with_borrow_mut(|kept| kept.as_mut().map(std::mem::take).unwrap_or_default())
}

/// Prints to stderr, or keeps `text` in the background
pub(crate) fn eprint_text(text: &str) {
    KEPT.with_borrow_mut(|kept| match kept {
        Some(kept) => kept.push_str(text),
        None => eprint!("{text}"),
    });
}

/// Stops sndbx with a message that isn't an error, printed as is
#[derive(Debug)]
pub struct Abort(pub String);

impl fmt::Display for Abort {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Abort {}

/// Ignores a closed pipe, like `head` leaves
pub fn print_line(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
}

pub fn print_status(message: &str) {
    print_line(&format!("==> {message}"));
}

/// Drops repeats, keeping the first-appearance order
pub fn unique<T: PartialEq>(items: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut unique = Vec::new();
    for item in items {
        if !unique.contains(&item) {
            unique.push(item);
        }
    }
    unique
}

/// Prints the error, if any, the way every sndbx binary does
pub fn exit_code(result: Result<()>) -> ExitCode {
    let Err(e) = result else {
        return ExitCode::SUCCESS;
    };

    match e.downcast_ref::<Abort>() {
        Some(Abort(message)) => eprintln!("{message}"),
        // `:#` adds the causes
        None => eprintln!("Error: {e:#}."),
    }
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_output_in_the_background() {
        std::thread::spawn(|| {
            assert!(!in_background());
            work_in_background();
            eprint_text("a\n");
            eprint_text("b\n");
            assert_eq!(take_kept_output(), "a\nb\n");
            assert_eq!(take_kept_output(), "");
        })
        .join()
        .unwrap();
    }

    #[test]
    fn unique_keeps_the_first_of_each() {
        assert_eq!(unique(["b", "a", "b", "c", "a"]), ["b", "a", "c"]);
        assert_eq!(unique(Vec::<&str>::new()), Vec::<&str>::new());
    }
}
