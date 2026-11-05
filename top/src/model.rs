//! What the dashboard shows, as read from docker, the settings and the proxy's log

use std::collections::{HashMap, VecDeque};

use jiff::Timestamp;
use sndbx::proxy::LogLine;
use sndbx::settings::Settings;

/// Lines kept from the proxy's log, also how many it starts with; the hosts' totals are theirs
pub const LOG_LINES: usize = 10_000;

pub struct Sandbox {
    pub name: String,
    /// The image's name in the settings, from the container's label
    pub image: String,
    /// Docker's, like `running` or `exited`
    pub state: String,
    pub started: Option<Timestamp>,
    pub workdir: String,
    pub mounts: Vec<Mount>,
    /// Published, like docker's `-p`
    pub ports: Vec<String>,
    pub networks: Vec<String>,
    /// On the proxy's internal network, what the proxy logs it as
    pub proxy_ip: Option<String>,
    /// The proxy's one, or one on another network for a sandbox that doesn't use the proxy
    pub ip: Option<String>,
    pub pids_limit: Option<u64>,
}

impl Sandbox {
    pub fn running(&self) -> bool {
        self.state == "running"
    }

    pub fn proxied(&self) -> bool {
        self.networks
            .iter()
            .any(|network| network == sndbx::proxy::NETWORK)
    }

    /// Whether the proxy logged `line` for this sandbox: from its address, since it started, as docker gives a gone
    /// container's address to the next one. Up to a second before, as the log has whole seconds.
    pub fn sent(&self, line: &LogLine) -> bool {
        self.proxy_ip.as_deref() == Some(line.client.as_str())
            && self.started.is_some_and(|started| {
                line.time
                    .checked_add(jiff::SignedDuration::from_secs(1))
                    .is_ok_and(|time| time >= started)
            })
    }
}

pub struct Mount {
    /// On the host
    pub source: String,
    /// In the container
    pub destination: String,
}

#[derive(Clone, Default)]
pub struct Stats {
    pub cpu: String,
    pub memory: String,
    /// Of the container's limit, or the host's memory
    pub memory_percent: String,
    pub pids: String,
    /// Bytes the sandbox received and sent on its networks since it started
    pub received: String,
    pub sent: String,
}

/// A built image, from `docker image ls`
pub struct ImageInfo {
    pub size: String,
    pub created: Option<Timestamp>,
}

/// What a refresh reads, all at once so the views agree
#[derive(Default)]
pub struct Snapshot {
    /// Why docker can't be read, if it can't
    pub docker_error: Option<String>,
    pub sandboxes: Vec<Sandbox>,
    pub proxy_up: bool,
    /// By `repository:tag`
    pub images: HashMap<String, ImageInfo>,
    pub settings: Option<Settings>,
    pub settings_error: Option<String>,
    /// The images `images_dir` has, without base
    pub buildable: Vec<String>,
}

impl Snapshot {
    pub fn image(&self, tag: &str) -> Option<&ImageInfo> {
        self.images.get(&normalize_tag(tag))
    }

    /// The sandbox the proxy logged `line` for, if it's still there
    pub fn sandbox_for(&self, line: &LogLine) -> Option<&Sandbox> {
        self.sandboxes.iter().find(|sandbox| sandbox.sent(line))
    }
}

/// With docker's default `latest` tag when it has none
pub fn normalize_tag(tag: &str) -> String {
    let name = tag.rsplit('/').next().unwrap_or(tag);
    if name.contains(':') || tag.contains('@') {
        tag.to_string()
    } else {
        format!("{tag}:latest")
    }
}

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Traffic {
    pub requests: u64,
    pub denied: u64,
    /// From the sandbox
    pub bytes_in: u64,
    /// To the sandbox
    pub bytes_out: u64,
}

/// The proxy's recent requests
#[derive(Default)]
pub struct ProxyLog {
    pub lines: VecDeque<LogLine>,
}

impl ProxyLog {
    /// Empty, for reading the log from its start again
    pub fn clear(&mut self) {
        self.lines.clear();
    }

    pub fn push(&mut self, line: LogLine) {
        if self.lines.len() == LOG_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    /// The hosts the sandbox asked for in the kept lines, most often first; not the lines without one
    pub fn hosts_of(&self, sandbox: &Sandbox) -> Vec<(&str, Traffic)> {
        let mut totals: HashMap<&str, Traffic> = HashMap::new();
        let lines = self
            .lines
            .iter()
            .filter(|line| !line.host.is_empty() && sandbox.sent(line));
        for line in lines {
            totals.entry(line.host.as_str()).or_default().add(line);
        }
        let mut hosts: Vec<(&str, Traffic)> = totals.into_iter().collect();
        hosts.sort_by(|(a_host, a), (b_host, b)| {
            b.requests.cmp(&a.requests).then(a_host.cmp(b_host))
        });
        hosts
    }

    /// How many of the kept lines the proxy denied
    pub fn denied(&self) -> usize {
        self.lines.iter().filter(|line| line.denied()).count()
    }
}

impl Traffic {
    fn add(&mut self, line: &LogLine) {
        self.requests += 1;
        self.bytes_in += line.bytes_in;
        self.bytes_out += line.bytes_out;
        if line.denied() {
            self.denied += 1;
        }
    }

    pub fn only_denied(&self) -> bool {
        self.denied == self.requests
    }
}

/// Like `3d`, `2h13m`, `14m` or `5s`
pub fn age(since: Timestamp, now: Timestamp) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    let seconds = now.duration_since(since).as_secs().max(0);
    match seconds {
        DAY.. => format!("{}d", seconds / DAY),
        HOUR.. => format!("{}h{:02}m", seconds / HOUR, seconds % HOUR / MINUTE),
        MINUTE.. => format!("{}m", seconds / MINUTE),
        _ => format!("{seconds}s"),
    }
}

/// Like `512B`, `1.2K` or `48M`, with a decimal below 10
pub fn bytes(count: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    #[allow(clippy::cast_precision_loss)]
    let mut value = count as f64;
    let mut unit = 0;
    // From 999.5 on, as it would round to 1000
    while value >= 999.5 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    let decimals = usize::from(unit > 0 && value < 9.95);
    format!("{value:.decimals$}{}", UNITS[unit])
}

/// Docker's `612MiB`, `1.9GB` or `3.4kB` as `612M`, `1.9G` or `3.4K`, keeping `B` for plain bytes
pub fn short_size(size: &str) -> String {
    let size = size.trim();
    size.strip_suffix("iB")
        .or_else(|| {
            size.strip_suffix('B')
                .filter(|rest| rest.ends_with(|c: char| c.is_ascii_alphabetic()))
        })
        .unwrap_or(size)
        .replace('k', "K")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(client: &str, result: &str, host: &str) -> LogLine {
        line_at("2026-11-05T12:41:03+0000", client, result, host)
    }

    fn line_at(time: &str, client: &str, result: &str, host: &str) -> LogLine {
        LogLine::parse(&format!(
            "{time} {client} {result} CONNECT {host}:443 dur=1ms in=10 out=100"
        ))
        .unwrap()
    }

    fn sandbox(ip: &str, started: &str) -> Sandbox {
        Sandbox {
            name: "a".to_string(),
            image: "claude".to_string(),
            state: "running".to_string(),
            started: started.parse().ok(),
            workdir: String::new(),
            mounts: Vec::new(),
            ports: Vec::new(),
            networks: vec![sndbx::proxy::NETWORK.to_string()],
            proxy_ip: Some(ip.to_string()),
            ip: Some(ip.to_string()),
            pids_limit: None,
        }
    }

    #[test]
    fn takes_a_sandboxs_lines_from_its_address_since_it_started() {
        let started = sandbox("1.1.1.1", "2026-11-05T12:00:00.5Z");
        let sent = |time: &str, client: &str| {
            started.sent(&line_at(time, client, "TCP_TUNNEL/200", "a.com"))
        };
        assert!(sent("2026-11-05T12:30:00+0000", "1.1.1.1"));
        // The log's second may be the one it started in
        assert!(sent("2026-11-05T12:00:00+0000", "1.1.1.1"));
        // From a container that had the address before
        assert!(!sent("2026-11-05T11:59:58+0000", "1.1.1.1"));
        assert!(!sent("2026-11-05T12:30:00+0000", "2.2.2.2"));
        let unknown_start = Sandbox {
            started: None,
            ..sandbox("1.1.1.1", "")
        };
        assert!(!unknown_start.sent(&line("1.1.1.1", "TCP_TUNNEL/200", "a.com")));
    }

    #[test]
    fn counts_denied_lines() {
        let mut log = ProxyLog::default();
        log.push(line("1.1.1.1", "TCP_TUNNEL/200", "a.com"));
        log.push(line("1.1.1.1", "TCP_DENIED/403", "b.com"));
        log.push(line("2.2.2.2", "TCP_DENIED/403", "b.com"));
        assert_eq!(log.denied(), 2);
        log.clear();
        assert_eq!(log.denied(), 0);
    }

    #[test]
    fn totals_a_sandboxs_hosts() {
        let mut log = ProxyLog::default();
        log.push(line("1.1.1.1", "TCP_DENIED/403", "a.com"));
        log.push(line("1.1.1.1", "TCP_TUNNEL/200", "b.com"));
        log.push(line("1.1.1.1", "TCP_TUNNEL/200", "c.com"));
        log.push(line("1.1.1.1", "TCP_TUNNEL/200", "c.com"));
        log.push(line("1.1.1.1", "TCP_DENIED/403", "c.com"));
        log.push(line("2.2.2.2", "TCP_TUNNEL/200", "d.com"));
        log.push(line_at(
            "2026-11-05T11:00:00+0000",
            "1.1.1.1",
            "TCP_TUNNEL/200",
            "e.com",
        ));
        log.push(
            LogLine::parse(
                "2026-11-05T12:41:03+0000 1.1.1.1 NONE_NONE/000 - error:transaction-end-before-headers dur=0ms \
                 in=0 out=0",
            )
            .unwrap(),
        );

        let hosts: Vec<(&str, u64, u64)> = log
            .hosts_of(&sandbox("1.1.1.1", "2026-11-05T12:00:00Z"))
            .into_iter()
            .map(|(host, traffic)| (host, traffic.requests, traffic.denied))
            .collect();
        assert_eq!(hosts, [("c.com", 3, 1), ("a.com", 1, 1), ("b.com", 1, 0)]);
        assert_eq!(
            log.hosts_of(&sandbox("3.3.3.3", "2026-11-05T12:00:00Z")),
            Vec::<(&str, Traffic)>::new()
        );
    }

    #[test]
    fn keeps_the_last_lines() {
        let mut log = ProxyLog::default();
        for _ in 0..=LOG_LINES {
            log.push(line("1.1.1.1", "TCP_TUNNEL/200", "a.com"));
        }
        assert_eq!(log.lines.len(), LOG_LINES);
        assert_eq!(
            log.hosts_of(&sandbox("1.1.1.1", "2026-11-05T12:00:00Z"))[0]
                .1
                .requests,
            LOG_LINES as u64
        );
    }

    #[test]
    fn formats() {
        let now: Timestamp = "2026-11-05T12:00:00Z".parse().unwrap();
        let ago = |seconds: i64| {
            now.checked_sub(jiff::SignedDuration::from_secs(seconds))
                .unwrap()
        };
        assert_eq!(age(ago(5), now), "5s");
        assert_eq!(age(ago(14 * 60 + 5), now), "14m");
        assert_eq!(age(ago(2 * 3600 + 13 * 60), now), "2h13m");
        assert_eq!(age(ago(3 * 86400 + 5), now), "3d");
        assert_eq!(age(ago(-5), now), "0s");

        assert_eq!(bytes(512), "512B");
        assert_eq!(bytes(1200), "1.2K");
        assert_eq!(bytes(48_000), "48K");
        assert_eq!(bytes(3_100_000), "3.1M");
        assert_eq!(bytes(999_999), "1.0M");

        assert_eq!(short_size("612MiB"), "612M");
        assert_eq!(short_size("1.9GB"), "1.9G");
        assert_eq!(short_size("3.4kB"), "3.4K");
        assert_eq!(short_size("512B"), "512B");
        assert_eq!(short_size("0B"), "0B");
    }

    #[test]
    fn tags() {
        assert_eq!(normalize_tag("sndbx:claude"), "sndbx:claude");
        assert_eq!(normalize_tag("my-claude"), "my-claude:latest");
        assert_eq!(normalize_tag("localhost:5000/x"), "localhost:5000/x:latest");
    }
}
