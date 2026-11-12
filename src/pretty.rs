//! What the `--pretty` logs share: times, columns, colors and printing a followed log's lines

use std::io::{self, BufRead, BufReader, IsTerminal, Read, Write};
use std::process::{Child, ExitStatus};

use anyhow::Context;

use crate::Result;

pub const TIME_FORMAT: &str = "%F %T";
pub const COLUMN_GAP: &str = "  ";
pub const LINE_END: &str = "\n";

const NEWLINE: u8 = b'\n';
const NO_COLOR_ENV: &str = "NO_COLOR";
// Starts a color, then a reset, ANSI's Select Graphic Rendition
const SGR_START: &str = "\x1b[";
const SGR_END: char = 'm';
const SGR_RESET: &str = "\x1b[0m";

/// Why printing stopped
enum End {
    LogEnded,
    /// Like `head` leaves it
    StdoutClosed,
}

/// Prints the log `child` follows to its piped stdout, each line formatted and ended with `line_end`. Checks how
/// `child` exited if the log ends, and stops it if stdout closes.
pub fn follow(
    mut child: Child,
    line_end: &str,
    format: impl FnMut(&str) -> String,
    read_error: &str,
    check_exit: impl FnOnce(io::Result<ExitStatus>) -> Result<()>,
) -> Result<()> {
    let output = child.stdout.take().expect("piped stdout");
    match print_lines(output, line_end, format).context(read_error.to_string()) {
        Ok(End::LogEnded) => check_exit(child.wait()),
        Ok(End::StdoutClosed) => {
            stop(&mut child);
            Ok(())
        }
        Err(e) => {
            stop(&mut child);
            Err(e)
        }
    }
}

/// Prints each of the lines formatted, ending it with `line_end`; `Err` only if they can't be read
fn print_lines(
    lines: impl Read,
    line_end: &str,
    mut format: impl FnMut(&str) -> String,
) -> io::Result<End> {
    let mut stdout = io::stdout().lock();
    for line in BufReader::new(lines).split(NEWLINE) {
        let line = line?;
        let line = format(&String::from_utf8_lossy(&line));
        if write!(stdout, "{line}{line_end}").is_err() {
            return Ok(End::StdoutClosed);
        }
    }
    Ok(End::LogEnded)
}

/// Kills the process following the log
fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[derive(Clone, Copy)]
pub enum Color {
    Dim,
    BrightBlue,
    Cyan,
    BrightCyan,
    Magenta,
    BrightMagenta,
    Green,
    BrightRed,
    Yellow,
}

impl Color {
    /// Its Select Graphic Rendition parameter
    fn parameter(self) -> &'static str {
        match self {
            Self::Dim => "2",
            Self::BrightBlue => "94",
            Self::Cyan => "36",
            Self::BrightCyan => "96",
            Self::Magenta => "35",
            Self::BrightMagenta => "95",
            Self::Green => "32",
            Self::BrightRed => "91",
            Self::Yellow => "33",
        }
    }
}

#[derive(Clone, Copy)]
pub struct Painter {
    pub enabled: bool,
}

impl Painter {
    /// On a terminal, unless `NO_COLOR` is set and not empty
    pub fn for_stdout() -> Self {
        let no_color = std::env::var_os(NO_COLOR_ENV).is_some_and(|value| !value.is_empty());
        Self {
            enabled: io::stdout().is_terminal() && !no_color,
        }
    }

    pub fn paint(self, text: &str, color: Option<Color>) -> String {
        match color {
            Some(color) if self.enabled => {
                format!("{SGR_START}{}{SGR_END}{text}{SGR_RESET}", color.parameter())
            }
            _ => text.to_string(),
        }
    }
}
