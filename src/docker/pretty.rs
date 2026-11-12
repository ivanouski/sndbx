//! `logs commands --pretty`: the command log in local times, with a row for each option, in color on a terminal.
//! Knows of docker's arguments only that options start with `-` and may have their value after `=`, and which of
//! the flags sndbx writes have no value.

use std::iter;
use std::process::Child;

use anyhow::{Context, bail};
use jiff::Timestamp;
use jiff::tz::TimeZone;

use super::LOG_TIME_FORMAT;
use crate::Result;
use crate::paths::shell_split;
use crate::pretty::{self, COLUMN_GAP, Color, LINE_END, Painter, TIME_FORMAT};

const OPTION_PREFIX: char = '-';
// The flags without a value sndbx writes, which would take the container, network or image after them
const FLAGS_WITHOUT_VALUE: [&str; 9] = [
    "--rm",
    "-d",
    "-i",
    "-it",
    "-ai",
    "-f",
    "--volumes",
    "--internal",
    "--no-cache",
];
const VALUE_SEPARATOR: char = '=';
const WORD_SEPARATOR: char = ' ';
const FOLLOW_ERROR: &str = "can't follow the command log";

/// Prints the log `tail` follows to its piped stdout
pub fn follow(tail: Child) -> Result<()> {
    let formatter = Formatter {
        time_zone: TimeZone::system(),
        painter: Painter::for_stdout(),
    };
    pretty::follow(
        tail,
        LINE_END,
        |line| formatter.format(line),
        FOLLOW_ERROR,
        |status| {
            // tail printed its own error
            if !status.context(FOLLOW_ERROR)?.success() {
                bail!(FOLLOW_ERROR);
            }
            Ok(())
        },
    )
}

/// A logged command's words, unescaped
struct LoggedCommand {
    /// The words before the first option, like `docker run`
    command: Vec<String>,
    options: Vec<CommandOption>,
    /// The words after the options, like the image and its command
    trailing: Vec<String>,
}

/// `--key=value`, `-k value`, or `-k` without a value: a flag sndbx writes, or before another option or none
struct CommandOption {
    key: String,
    value: Option<String>,
}

impl LoggedCommand {
    /// An option takes the word after it unless it's a flag sndbx writes, so one of yours without a value before
    /// the image takes the image
    fn parse(words: Vec<String>) -> Self {
        let mut words = words.into_iter().peekable();
        let command = iter::from_fn(|| words.next_if(|word| !is_option(word))).collect();
        let mut options = Vec::new();
        while let Some(word) = words.next_if(|word| is_option(word)) {
            let option = match word.split_once(VALUE_SEPARATOR) {
                Some((key, value)) => CommandOption {
                    key: key.to_string(),
                    value: Some(value.to_string()),
                },
                None if FLAGS_WITHOUT_VALUE.contains(&word.as_str()) => CommandOption {
                    key: word,
                    value: None,
                },
                None => CommandOption {
                    value: words.next_if(|word| !is_option(word)),
                    key: word,
                },
            };
            options.push(option);
        }
        Self {
            command,
            options,
            trailing: words.collect(),
        }
    }

    /// The values of each key, in the order the keys first appear
    fn values_by_key(&self) -> Vec<(&str, Vec<&str>)> {
        let mut groups: Vec<(&str, Vec<&str>)> = Vec::new();
        for option in &self.options {
            let Some(value) = &option.value else {
                continue;
            };
            match groups.iter_mut().find(|(key, _)| *key == option.key) {
                Some((_, values)) => values.push(value),
                None => groups.push((&option.key, vec![value])),
            }
        }
        groups
    }
}

/// A lone `-`, like stdin, and a negative number are values
fn is_option(word: &str) -> bool {
    word.strip_prefix(OPTION_PREFIX)
        .and_then(|rest| rest.chars().next())
        .is_some_and(|first| !first.is_ascii_digit())
}

struct Formatter {
    time_zone: TimeZone,
    painter: Painter,
}

impl Formatter {
    /// The line as is without a time, the command as logged if it doesn't parse
    fn format(&self, line: &str) -> String {
        let Some((time, command)) = line.split_once(WORD_SEPARATOR) else {
            return line.to_string();
        };
        let Ok(time) = Timestamp::strptime(LOG_TIME_FORMAT, time) else {
            return line.to_string();
        };
        let time = time
            .to_zoned(self.time_zone.clone())
            .strftime(TIME_FORMAT)
            .to_string();
        let time_width = time.chars().count();
        let time = self.painter.paint(&time, Some(Color::Dim));
        let command = match shell_split(command) {
            Some(words) => self.rows(&LoggedCommand::parse(words), time_width),
            None => command.to_string(),
        };
        format!("{time}{WORD_SEPARATOR}{command}")
    }

    /// The command, its options without values and the trailing words, then a row for each key with values
    fn rows(&self, command: &LoggedCommand, time_width: usize) -> String {
        let groups = command.values_by_key();
        let width = groups
            .iter()
            .map(|(key, _)| key.chars().count())
            .max()
            .unwrap_or_default();
        // Under the command's second word, like `run`, so every command's rows line up. Each followed by a separator.
        let indent: usize = iter::once(time_width)
            .chain(command.command.first().map(|word| word.chars().count()))
            .map(|width| width + WORD_SEPARATOR.len_utf8())
            .sum();
        let indent = format!("{:indent$}", "");
        let mut lines = vec![self.header(command)];
        for (key, values) in groups {
            let mut key = self.key(&format!("{key:<width$}"));
            for value in values {
                let value = self.value(value);
                lines.push(format!("{indent}{key}{COLUMN_GAP}{value}"));
                key = format!("{:width$}", "");
            }
        }
        lines.join(LINE_END)
    }

    /// The command, the options without values, then the trailing words: the container or image, then its command
    fn header(&self, command: &LoggedCommand) -> String {
        let flags = command
            .options
            .iter()
            .filter(|option| option.value.is_none())
            .map(|option| self.key(&option.key));
        let words: Vec<String> = command
            .command
            .iter()
            .map(|word| self.painter.paint(word, Some(Color::BrightMagenta)))
            .chain(flags)
            .chain(command.trailing.iter().enumerate().map(|(index, word)| {
                // The container or image stands out from the command it runs
                let color = (index == 0).then_some(Color::Cyan);
                self.painter.paint(word, color)
            }))
            .collect();
        words.join(&WORD_SEPARATOR.to_string())
    }

    fn key(&self, key: &str) -> String {
        self.painter.paint(key, Some(Color::BrightBlue))
    }

    /// With the name before the first `=` in color, like an environment variable's
    fn value(&self, value: &str) -> String {
        match value.split_once(VALUE_SEPARATOR) {
            Some((name, rest)) => {
                let name = self.painter.paint(name, Some(Color::Yellow));
                format!("{name}{VALUE_SEPARATOR}{rest}")
            }
            None => value.to_string(),
        }
    }
}

#[cfg(test)]
mod tests;
