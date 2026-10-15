//! Path and shell helpers. Host paths are `PathBuf`s; paths inside the container, like mount targets,
//! are `String`s that only docker reads, joined with `join`.

use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

pub fn home() -> PathBuf {
    std::env::home_dir().unwrap_or_default()
}

pub fn current_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
}

/// Expands `~`, resolves `path` against `base`, the current directory by default,
/// and removes `.`, `..` and repeated slashes without following symlinks.
pub fn expand_path(path: impl AsRef<Path>, base: Option<&Path>) -> PathBuf {
    let path = path.as_ref();
    let absolute = match path.strip_prefix("~") {
        Ok(rest) => home().join(rest),
        Err(_) => base.map_or_else(current_dir, |base| expand_path(base, None)).join(path),
    };
    normalize(&absolute)
}

/// An absolute path without `.`, `..` or repeated slashes, like `/a/c` for `a/./b/../c/`
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

/// Joins with one slash between parts, whatever slashes they have, like `/a/b` for `/a/` and `/b`
pub fn join(parts: &[&str]) -> String {
    let mut joined = String::new();
    for (index, part) in parts.iter().enumerate() {
        if index == 0 {
            joined.push_str(part);
        } else {
            let trimmed = joined.trim_end_matches('/').len();
            joined.truncate(trimmed);
            joined.push('/');
            joined.push_str(part.trim_start_matches('/'));
        }
    }
    joined
}

/// Whether both paths exist and are the same file, following symlinks
pub fn is_same_file(first: &Path, second: &Path) -> bool {
    match (first.metadata(), second.metadata()) {
        (Ok(first), Ok(second)) => first.dev() == second.dev() && first.ino() == second.ino(),
        _ => false,
    }
}

/// The words as a shell command line, each escaped with `shell_escape`
pub fn shell_join(words: &[&str]) -> String {
    words
        .iter()
        .map(|word| shell_escape(word))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A backslash before each character a shell treats specially, `''` for an empty word,
/// and a newline in single quotes
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
        assert_eq!(join(&["/home", "ubuntu", ".claude"]), "/home/ubuntu/.claude");
        assert_eq!(join(&["/home/", "/ubuntu/", "/x"]), "/home/ubuntu/x");
        assert_eq!(join(&["/workspace", ""]), "/workspace/");
    }

    #[test]
    fn shell_join_escapes() {
        let words = ["docker", "my dir", "", "a\nb", "x=$y", "/a:b@c,d+e"];
        assert_eq!(shell_join(&words), "docker my\\ dir '' a'\n'b x\\=\\$y /a:b@c,d+e");
    }
}
