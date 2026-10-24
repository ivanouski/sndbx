//! Path and shell helpers. Host paths are `PathBuf`s, container paths `String`s joined with `join`.

use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, bail};

use crate::Result;

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
    let absolute = match path.strip_prefix("~") {
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
mod tests {
    use super::*;

    #[test]
    fn expand_path_normalizes() {
        assert_eq!(expand_path("/a/./b/../c/", None), Path::new("/a/c"));
        assert_eq!(expand_path("//a/b", None), Path::new("/a/b"));
        assert_eq!(expand_path("/..", None), Path::new("/"));
        assert_eq!(expand_path("b", Some(Path::new("/a"))), Path::new("/a/b"));
        assert_eq!(expand_path("/b", Some(Path::new("/a"))), Path::new("/b"));
        assert_eq!(expand_path("~/x/..", None), home());
        assert_eq!(expand_path("~x", Some(Path::new("/a"))), Path::new("/a/~x"));
    }

    #[test]
    fn join_uses_one_slash() {
        assert_eq!(
            join(&["/home", "ubuntu", ".claude"]),
            "/home/ubuntu/.claude"
        );
        assert_eq!(join(&["/home/", "/ubuntu/", "/x"]), "/home/ubuntu/x");
        assert_eq!(join(&["/workspace", ""]), "/workspace/");
    }

    #[test]
    fn shell_join_escapes() {
        let words = ["docker", "my dir", "", "a\nb", "x=$y", "/a:b@c,d+e"];
        assert_eq!(
            shell_join(&words),
            "docker my\\ dir '' a'\n'b x\\=\\$y /a:b@c,d+e"
        );
    }
}
