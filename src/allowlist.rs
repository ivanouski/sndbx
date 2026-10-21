//! The hosts proxied sandboxes may reach, one per line in their own file

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use crate::Result;
use crate::settings;

pub fn path() -> PathBuf {
    settings::dir().join("allowed_hosts")
}

/// The allowed hosts. A missing file is an error rather than no hosts, which would block everything,
/// or no proxy, which would let everything through.
pub fn load() -> Result<Vec<String>> {
    let path = path();
    if !path.exists() {
        bail!(
            "no {}, list the hosts sandboxes behind the proxy may reach there, one per line, \
             or remove proxy_image from {}",
            path.display(),
            settings::path().display()
        );
    }
    let content = std::fs::read_to_string(&path).with_context(|| format!("can't read {}", path.display()))?;
    parse(&content, &path)
}

/// The hosts in `content`, one per line, without blank lines, `#` comments, repeated hosts and hosts a `.domain`
/// covers: squid ignores one of two overlapping hosts, which may be the `.domain`
fn parse(content: &str, path: &Path) -> Result<Vec<String>> {
    let mut hosts: Vec<String> = Vec::new();
    for (index, line) in content.lines().enumerate() {
        let host = line.split_once('#').map_or(line, |(host, _comment)| host).trim();
        if host.is_empty() {
            continue;
        }
        if !is_valid_host(host) {
            bail!(
                "invalid {} line {}: '{host}' is not a host name like .openai.com or pypi.org",
                path.display(),
                index + 1
            );
        }
        if !hosts.iter().any(|known| known == host) {
            hosts.push(host.to_string());
        }
    }
    Ok(hosts
        .iter()
        .filter(|host| !hosts.iter().any(|other| covers(other, host)))
        .cloned()
        .collect())
}

/// Whether `domain`, like `.a.com`, also allows the other host, like `a.com`, `x.a.com` or `.x.a.com`
fn covers(domain: &str, host: &str) -> bool {
    domain != host && domain.starts_with('.') && (host == &domain[1..] || host.ends_with(domain))
}

/// Lowercase alphanumeric labels joined by dots, `-` inside them, a leading dot for subdomains too,
/// so nothing else reaches the proxy's config
fn is_valid_host(host: &str) -> bool {
    host.strip_prefix('.').unwrap_or(host).split('.').all(|label| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        let content = "# models\n.openai.com\n\n  pypi.org  # packages\n.openai.com\n";
        assert_eq!(parse(content, Path::new("x")).unwrap(), [".openai.com", "pypi.org"]);
        assert_eq!(parse("", Path::new("x")).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn drops_hosts_a_domain_covers() {
        let content = "api.github.com\n.x.github.com\ngithub.com\n.github.com\nmygithub.com\npypi.org\n";
        assert_eq!(
            parse(content, Path::new("x")).unwrap(),
            [".github.com", "mygithub.com", "pypi.org"]
        );
    }

    #[test]
    fn invalid_hosts() {
        let message = parse("a.com\nopen ai.com\n", Path::new("x")).err().unwrap().to_string();
        assert_eq!(
            message,
            "invalid x line 2: 'open ai.com' is not a host name like .openai.com or pypi.org"
        );
        for invalid in [
            "Example.com",
            "a..com",
            "..a",
            "-a.com",
            "a-.com",
            "http_access",
            "a.com/x",
            "*.a.com",
        ] {
            assert!(!is_valid_host(invalid), "{invalid}");
        }
        for valid in ["localhost", "a.com", ".a.com", "x-1.a2.com", "1.2.3.4"] {
            assert!(is_valid_host(valid), "{valid}");
        }
    }
}
