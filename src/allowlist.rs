//! The hosts proxied sandboxes may reach, one per line

use std::path::{Path, PathBuf};

use anyhow::bail;

use crate::settings;
use crate::{Result, paths, unique};

pub fn path() -> PathBuf {
    settings::dir().join("allowed_hosts")
}

/// A missing file is an error: neither blocking nor allowing everything is a safe guess
pub fn load() -> Result<Vec<String>> {
    let path = path();
    if !path.exists() {
        bail!(
            "no {}, list the hosts sandboxes behind the proxy may reach there, one per line, \
             or remove proxy from {}",
            path.display(),
            settings::path().display()
        );
    }
    let content = paths::read_file(&path)?;
    parse(&content, &path)
}

/// Drops `#` comments, repeats and hosts a `.domain` covers, as squid ignores one of two overlapping hosts
fn parse(content: &str, path: &Path) -> Result<Vec<String>> {
    let mut hosts: Vec<String> = Vec::new();
    for (index, line) in content.lines().enumerate() {
        let host = line
            .split_once('#')
            .map_or(line, |(host, _comment)| host)
            .trim();
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
        hosts.push(host.to_string());
    }
    let hosts = unique(hosts);
    Ok(hosts
        .iter()
        .filter(|host| !hosts.iter().any(|other| covers(other, host)))
        .cloned()
        .collect())
}

/// Whether `domain`, like `.a.com`, covers `host`, like `a.com`, `x.a.com` or `.x.a.com`
fn covers(domain: &str, host: &str) -> bool {
    domain != host
        && domain
            .strip_prefix('.')
            .is_some_and(|parent| host == parent || host.ends_with(domain))
}

/// Strict, so nothing else reaches the proxy's config
fn is_valid_host(host: &str) -> bool {
    host.strip_prefix('.')
        .unwrap_or(host)
        .split('.')
        .all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

#[cfg(test)]
mod tests;
