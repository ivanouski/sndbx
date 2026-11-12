//! Path and shell helpers. Host paths are `PathBuf`s, container paths `String`s joined with `join`.

use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, bail};

use crate::Result;

/// `~`, the home directory at the start of a path
pub const HOME_SHORTHAND: &str = "~";
/// `~/`, the start of a path in the home; in `docker_args` only this form expands
pub const HOME_PREFIX: &str = "~/";

pub fn home() -> PathBuf {
    std::env::home_dir().unwrap_or_default()
}

/// `$variable` if absolute, else `~/fallback`, per the XDG spec
pub fn xdg_dir(variable: &str, fallback: &str) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .unwrap_or_else(|| home().join(fallback))
}

pub fn state_dir() -> PathBuf {
    xdg_dir("XDG_STATE_HOME", ".local/state").join("sndbx")
}

/// A relative home would make the XDG fallbacks and `~` relative to the current directory
pub fn check_home() -> Result<()> {
    if !home().is_absolute() {
        bail!("HOME is not set to an absolute path");
    }
    Ok(())
}

pub fn read_file(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("can't read {}", path.display()))
}

/// Writes in place, following symlinks and keeping permissions
pub fn write_file(path: &Path, content: &str) -> Result<()> {
    create_parent_dir(path)
        .and_then(|()| std::fs::write(path, content))
        .with_context(|| format!("can't write {}", path.display()))
}

pub fn create_parent_dir(path: &Path) -> std::io::Result<()> {
    path.parent().map_or(Ok(()), std::fs::create_dir_all)
}

pub fn current_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
}

/// Expands `~`, resolves against `base` or the current directory, and normalizes without following symlinks
pub fn expand_path(path: impl AsRef<Path>, base: Option<&Path>) -> PathBuf {
    let path = path.as_ref();
    let absolute = match path.strip_prefix(HOME_SHORTHAND) {
        Ok(rest) => home().join(rest),
        Err(_) => base
            .map_or_else(current_dir, |base| expand_path(base, None))
            .join(path),
    };
    normalize(&absolute)
}

/// Like `/a/c` for `a/./b/../c/`
pub fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::from("/");
    for component in path.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
            Component::Prefix(_) | Component::RootDir | Component::CurDir => {}
        }
    }
    normalized
}

/// For container paths, which are `String`s; host paths have `Path::is_absolute`
pub fn is_absolute(path: &str) -> bool {
    path.starts_with('/')
}

/// One slash between parts, like `/a/b` for `/a/` and `/b`
pub fn join(parts: &[&str]) -> String {
    let Some((first, rest)) = parts.split_first() else {
        return String::new();
    };
    rest.iter().fold(first.to_string(), |joined, part| {
        format!(
            "{}/{}",
            joined.trim_end_matches('/'),
            part.trim_start_matches('/')
        )
    })
}

/// Follows symlinks, false if either doesn't exist
pub fn is_same_file(first: &Path, second: &Path) -> bool {
    match (first.metadata(), second.metadata()) {
        (Ok(first), Ok(second)) => first.dev() == second.dev() && first.ino() == second.ino(),
        _ => false,
    }
}

pub fn shell_join(words: &[&str]) -> String {
    words
        .iter()
        .map(|word| shell_escape(word))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The words `shell_join` joined: backslashes and single quotes; `None` for an unclosed quote or a trailing
/// backslash
pub fn shell_split(line: &str) -> Option<Vec<String>> {
    const QUOTE: char = '\'';
    const ESCAPE: char = '\\';

    let mut words = Vec::new();
    // A quote starts a word even if it's empty
    let mut word: Option<String> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            ESCAPE => word.get_or_insert_default().push(chars.next()?),
            QUOTE => {
                let word = word.get_or_insert_default();
                loop {
                    match chars.next()? {
                        QUOTE => break,
                        c => word.push(c),
                    }
                }
            }
            c if c.is_whitespace() => words.extend(word.take()),
            c => word.get_or_insert_default().push(c),
        }
    }
    words.extend(word);
    Some(words)
}

/// Backslashes special characters; `''` for an empty word, newlines in single quotes
fn shell_escape(word: &str) -> String {
    if word.is_empty() {
        return "''".to_string();
    }

    let mut escaped = String::with_capacity(word.len());
    for c in word.chars() {
        if c == '\n' {
            escaped.push_str("'\n'");
        } else {
            if !(c.is_ascii_alphanumeric() || "_-.,:+/@".contains(c)) {
                escaped.push('\\');
            }
            escaped.push(c);
        }
    }
    escaped
}

#[cfg(test)]
mod tests;
