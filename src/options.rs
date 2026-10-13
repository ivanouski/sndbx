//! A small clone of the parts of Ruby's `OptionParser` that sndbx uses, from when sndbx was written
//! in Ruby, so it still accepts the same arguments, prints the same help and fails with the same messages.
//!
//! Like `OptionParser` it takes long options as `--name=VALUE` or `--name VALUE`, abbreviated to
//! any unique prefix, and short ones as `-xVALUE` or `-x VALUE`, with flags clustered as `-kl`.

use std::collections::VecDeque;

use crate::{Error, Result, error};

const SUMMARY_INDENT: &str = "    ";
const SUMMARY_WIDTH: usize = 32;

pub struct Switch {
    pub short: char,
    pub long: &'static str,
    /// The argument's name in the help, e.g. `NAME` for `--workdir=NAME`; `None` for flags
    pub arg: Option<&'static str>,
    pub description: Vec<String>,
}

impl Switch {
    pub fn flag(short: char, long: &'static str, description: &[&str]) -> Self {
        Self {
            short,
            long,
            arg: None,
            description: description.iter().map(ToString::to_string).collect(),
        }
    }

    pub fn with_arg(short: char, long: &'static str, arg: &'static str, description: &[&str]) -> Self {
        Self {
            short,
            long,
            arg: Some(arg),
            description: description.iter().map(ToString::to_string).collect(),
        }
    }

    pub fn help() -> Self {
        Self::flag('h', "help", &["Print help"])
    }

    fn summary(&self) -> String {
        let left = match self.arg {
            Some(arg) => format!("-{}, --{}={arg}", self.short, self.long),
            None => format!("-{}, --{}", self.short, self.long),
        };

        let mut lines = Vec::new();
        let mut descriptions = self.description.iter();
        if left.len() > SUMMARY_WIDTH {
            lines.push(format!("{SUMMARY_INDENT}{left}"));
        } else if let Some(first) = descriptions.next() {
            lines.push(format!("{SUMMARY_INDENT}{left:<SUMMARY_WIDTH$} {first}"));
        } else {
            lines.push(format!("{SUMMARY_INDENT}{left}"));
        }
        for description in descriptions {
            lines.push(format!("{SUMMARY_INDENT}{:SUMMARY_WIDTH$} {description}", ""));
        }

        lines.join("\n")
    }
}

pub enum Event {
    Opt {
        long: &'static str,
        value: Option<String>,
        /// The option as given, for error messages, e.g. `-n x` or `--lines=x`
        given: String,
    },
    Arg(String),
    /// `--`, the remaining arguments are left in the parser
    End,
}

pub struct Parser {
    banner: String,
    separators: Vec<String>,
    switches: Vec<Switch>,
    args: VecDeque<String>,
    /// The rest of a short option cluster, e.g. `l` after `-k` in `-kl`
    cluster: Option<String>,
}

impl Parser {
    pub fn new(args: Vec<String>, banner: &str, separators: &[String], switches: Vec<Switch>) -> Self {
        Self {
            banner: banner.to_string(),
            separators: separators.to_vec(),
            switches,
            args: args.into(),
            cluster: None,
        }
    }

    pub fn help(&self) -> String {
        let mut help = format!("{}\n", self.banner);
        for line in &self.separators {
            help.push_str(line);
            help.push('\n');
        }
        for switch in &self.switches {
            help.push_str(&switch.summary());
            help.push('\n');
        }
        help
    }

    /// Puts an argument back, to be read next
    pub fn unshift(&mut self, arg: String) {
        self.args.push_front(arg);
    }

    /// The arguments not read yet
    pub fn rest(&mut self) -> Vec<String> {
        self.args.drain(..).collect()
    }

    pub fn next_event(&mut self) -> Result<Option<Event>> {
        if let Some(cluster) = self.cluster.take() {
            return self.short(&cluster).map(Some);
        }

        let Some(arg) = self.args.pop_front() else {
            return Ok(None);
        };

        if arg == "--" {
            Ok(Some(Event::End))
        } else if let Some(long) = arg.strip_prefix("--") {
            self.long(&arg, long).map(Some)
        } else if arg.len() > 1 && arg.starts_with('-') {
            self.short(&arg[1..]).map(Some)
        } else {
            Ok(Some(Event::Arg(arg)))
        }
    }

    /// `body` is the option without its dashes, e.g. `lines=5`
    fn long(&mut self, arg: &str, body: &str) -> Result<Event> {
        let (name, value) = match body.split_once('=') {
            Some((name, value)) => (name, Some(value.to_string())),
            None => (body, None),
        };

        let Some(index) = self.find_long(name, arg)? else {
            // OptionParser's built-in --version, for commands that don't define one
            if !name.is_empty() && "version".starts_with(name) {
                return Err(Error::Abort(String::from("sndbx: version unknown")));
            }
            return error(format!("invalid option: {arg}"));
        };
        let switch = &self.switches[index];

        if switch.arg.is_none() {
            if value.is_some() {
                return error(format!("needless argument: {arg}"));
            }
            return Ok(Event::Opt {
                long: switch.long,
                value: None,
                given: arg.to_string(),
            });
        }

        let long = switch.long;
        match value {
            Some(value) => Ok(Event::Opt {
                long,
                value: Some(value),
                given: arg.to_string(),
            }),
            None => match self.args.pop_front() {
                Some(value) => Ok(Event::Opt {
                    long,
                    given: format!("{arg} {value}"),
                    value: Some(value),
                }),
                None => error(format!("missing argument: {arg}")),
            },
        }
    }

    fn find_long(&self, name: &str, arg: &str) -> Result<Option<usize>> {
        if let Some(index) = self.switches.iter().position(|switch| switch.long == name) {
            return Ok(Some(index));
        }

        let candidates: Vec<usize> = (0..self.switches.len())
            .filter(|&index| self.switches[index].long.starts_with(name))
            .collect();
        match candidates.as_slice() {
            [] => Ok(None),
            [index] => Ok(Some(*index)),
            _ => error(format!("ambiguous option: {arg}")),
        }
    }

    /// `cluster` is one or more short options without the dash, e.g. `kl` or `nx`
    fn short(&mut self, cluster: &str) -> Result<Event> {
        let mut chars = cluster.chars();
        let Some(letter) = chars.next() else {
            return error("invalid option: -");
        };
        let rest = chars.as_str();

        let Some(switch) = self.switches.iter().find(|switch| switch.short == letter) else {
            return error(format!("invalid option: -{letter}"));
        };
        let long = switch.long;

        if switch.arg.is_none() {
            if !rest.is_empty() {
                self.cluster = Some(rest.to_string());
            }
            return Ok(Event::Opt {
                long,
                value: None,
                given: format!("-{letter}"),
            });
        }

        if !rest.is_empty() {
            return Ok(Event::Opt {
                long,
                value: Some(rest.to_string()),
                given: format!("-{letter}{rest}"),
            });
        }
        match self.args.pop_front() {
            Some(value) => Ok(Event::Opt {
                long,
                given: format!("-{letter} {value}"),
                value: Some(value),
            }),
            None => error(format!("missing argument: -{letter}")),
        }
    }
}

/// Converts an option's value like `OptionParser`'s `Integer`: decimal, `0b` binary, `0` octal
/// or `0x` hexadecimal, with an optional sign and single underscores between digits.
pub fn integer(value: &str, given: &str) -> Result<i128> {
    parse_integer(value).map_or_else(|| error(format!("invalid argument: {given}")), Ok)
}

fn parse_integer(value: &str) -> Option<i128> {
    let (negative, unsigned) = match value.as_bytes().first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    let lower = unsigned.to_ascii_lowercase();

    let (radix, digits) = if let Some(digits) = lower.strip_prefix("0b") {
        (2, digits)
    } else if let Some(digits) = lower.strip_prefix("0x") {
        (16, digits)
    } else if lower == "0" {
        (10, "0")
    } else if let Some(digits) = lower.strip_prefix('0') {
        (8, digits)
    } else {
        (10, lower.as_str())
    };

    // Digits, with single underscores only between them
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
        return None;
    }

    let mut number: i128 = 0;
    for c in digits.chars().filter(|&c| c != '_') {
        let digit = i128::from(c.to_digit(radix)?);
        number = number.checked_mul(i128::from(radix))?.checked_add(digit)?;
    }

    Some(if negative { -number } else { number })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parser(args: &[&str]) -> Parser {
        Parser::new(
            args.iter().map(ToString::to_string).collect(),
            "Usage: test",
            &[],
            vec![
                Switch::with_arg('v', "volume", "SRC", &["Mount SRC"]),
                Switch::flag('k', "keep", &["Keep"]),
                Switch::flag('l', "list", &["List"]),
                Switch::flag('L', "lines", &["Lines"]),
            ],
        )
    }

    fn events(args: &[&str]) -> Result<Vec<String>> {
        let mut parser = parser(args);
        let mut events = Vec::new();
        while let Some(event) = parser.next_event()? {
            events.push(match event {
                Event::Opt { long, value, given } => format!("{long}={value:?} ({given})"),
                Event::Arg(arg) => arg,
                Event::End => {
                    events.push(String::from("--"));
                    events.extend(parser.rest());
                    break;
                }
            });
        }
        Ok(events)
    }

    fn message(result: Result<Vec<String>>) -> String {
        match result {
            Err(Error::Message(message) | Error::Abort(message)) => message,
            Ok(events) => panic!("expected an error, got {events:?}"),
        }
    }

    #[test]
    fn long_options() {
        assert_eq!(
            events(&["--volume=a", "--volume", "b", "--vol", "c", "--keep", "x"]).unwrap(),
            [
                "volume=Some(\"a\") (--volume=a)",
                "volume=Some(\"b\") (--volume b)",
                "volume=Some(\"c\") (--vol c)",
                "keep=None (--keep)",
                "x"
            ]
        );
    }

    #[test]
    fn short_options() {
        assert_eq!(
            events(&["-kl", "-va", "-v", "-b", "-", "--", "-k"]).unwrap(),
            [
                "keep=None (-k)",
                "list=None (-l)",
                "volume=Some(\"a\") (-va)",
                "volume=Some(\"-b\") (-v -b)",
                "-",
                "--",
                "-k"
            ]
        );
    }

    #[test]
    fn errors() {
        assert_eq!(message(events(&["--bogus"])), "invalid option: --bogus");
        assert_eq!(message(events(&["-kx"])), "invalid option: -x");
        assert_eq!(message(events(&["-v"])), "missing argument: -v");
        assert_eq!(message(events(&["--vol"])), "missing argument: --vol");
        assert_eq!(message(events(&["--keep=x"])), "needless argument: --keep=x");
        assert_eq!(message(events(&["--li"])), "ambiguous option: --li");
        assert_eq!(message(events(&["--ver"])), "sndbx: version unknown");
    }

    #[test]
    fn integers() {
        assert_eq!(parse_integer("10"), Some(10));
        assert_eq!(parse_integer("-0"), Some(0));
        assert_eq!(parse_integer("+3"), Some(3));
        assert_eq!(parse_integer("1_000"), Some(1000));
        assert_eq!(parse_integer("010"), Some(8));
        assert_eq!(parse_integer("0b11"), Some(3));
        assert_eq!(parse_integer("0X1f"), Some(31));
        assert_eq!(parse_integer("99999999999999999999"), Some(99_999_999_999_999_999_999));
        for invalid in ["", "x", "1__0", "_1", "1_", "0o10", "1e3", " 5", "08", "-"] {
            assert_eq!(parse_integer(invalid), None, "{invalid}");
        }
    }
}
