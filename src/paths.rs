//! Ruby's path and shell helpers that sndbx relies on: `File.expand_path`, `File.join`,
//! `File.identical?` and `Shellwords.join`.

use std::os::unix::fs::MetadataExt;
use std::path::Path;

pub fn home() -> String {
    std::env::home_dir().map_or_else(String::new, |dir| dir.to_string_lossy().into_owned())
}

pub fn current_dir() -> String {
    std::env::current_dir().map_or_else(|_| String::from("/"), |dir| dir.to_string_lossy().into_owned())
}

/// Like Ruby's `File.expand_path`: expands `~`, resolves `path` against `base`, the current directory
/// by default, and removes `.`, `..` and repeated slashes without following symlinks.
pub fn expand_path(path: &str, base: Option<&str>) -> String {
    let absolute = if path == "~" || path.starts_with("~/") {
        format!("{}{}", home(), &path[1..])
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        let base = base.map_or_else(current_dir, |base| expand_path(base, None));
        format!("{base}/{path}")
    };

    normalize(&absolute)
}

fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }

    // Ruby keeps exactly two leading slashes, POSIX leaves their meaning to the system
    let root = if path.starts_with("//") && !path.starts_with("///") {
        "//"
    } else {
        "/"
    };
    format!("{root}{}", parts.join("/"))
}

/// Like Ruby's `File.join`: joins with one slash between parts, whatever slashes they have.
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

pub fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.is_empty() { "" } else { "/" };
    }
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

/// Like Ruby's `File.identical?`: whether both paths exist and are the same file, following symlinks.
pub fn identical(first: &str, second: &str) -> bool {
    match (std::fs::metadata(first), std::fs::metadata(second)) {
        (Ok(first), Ok(second)) => first.dev() == second.dev() && first.ino() == second.ino(),
        _ => false,
    }
}

pub fn exists(path: &str) -> bool {
    Path::new(path).exists()
}

pub fn is_dir(path: &str) -> bool {
    Path::new(path).is_dir()
}

/// Like Ruby's `Shellwords.join`
pub fn shell_join(words: &[String]) -> String {
    words
        .iter()
        .map(|word| shell_escape(word))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_escape(word: &str) -> String {
    if word.is_empty() {
        return String::from("''");
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
        assert_eq!(expand_path("/a/./b/../c/", None), "/a/c");
        assert_eq!(expand_path("//a/b", None), "//a/b");
        assert_eq!(expand_path("///a", None), "/a");
        assert_eq!(expand_path("/..", None), "/");
        assert_eq!(expand_path("b", Some("/a")), "/a/b");
        assert_eq!(expand_path("/b", Some("/a")), "/b");
    }

    #[test]
    fn join_uses_one_slash() {
        assert_eq!(join(&["/home", "ubuntu", ".claude"]), "/home/ubuntu/.claude");
        assert_eq!(join(&["/home/", "/ubuntu/", "/x"]), "/home/ubuntu/x");
        assert_eq!(join(&["/workspace", ""]), "/workspace/");
    }

    #[test]
    fn basename_of_paths() {
        assert_eq!(basename("/a/b"), "b");
        assert_eq!(basename("/a/b/"), "b");
        assert_eq!(basename("/"), "/");
    }

    #[test]
    fn shell_join_escapes_like_ruby() {
        let words = ["docker", "my dir", "", "a\nb", "x=$y", "/a:b@c,d+e"].map(String::from);
        assert_eq!(shell_join(&words), "docker my\\ dir '' a'\n'b x\\=\\$y /a:b@c,d+e");
    }
}
