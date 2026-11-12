//! Proxied sandboxes are on an internal network with no route out; the proxy, also on the default one, lets
//! through the allowed hosts

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use crate::docker::{self, ContainerState, container_state, network_internal};
use crate::{Result, paths};

mod pretty;

const NETWORK: &str = "sndbx-internal";
pub const CONTAINER: &str = "sndbx-proxy";
const PORT: u16 = 3128;
// What tools read the proxy from, and the hosts to bypass it for
const PROXY_ENV: [&str; 4] = ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"];
const PROXY_BYPASS_ENV: [&str; 2] = ["NO_PROXY", "no_proxy"];
const PROXY_BYPASS_HOSTS: &str = "localhost,127.0.0.1,::1";
const CONFIG_TARGET: &str = "/etc/squid/squid.conf";
const ACCESS_LOG: &str = "/var/log/squid/access.log";
// The log's time, like sndbx's log, which `--pretty` parses back
const LOG_TIME_FORMAT: &str = "%FT%T%z";
// Prefixes and the unit of the log's last fields
const DURATION_FIELD: &str = "dur=";
const DURATION_UNIT: &str = "ms";
const SENT_FIELD: &str = "in=";
const RECEIVED_FIELD: &str = "out=";
// Squid's result for denied requests, before the HTTP status
const DENIED_RESULT: &str = "TCP_DENIED";
// Squid's result and the HTTP status, in requests' lines
const STATUS_SEPARATOR: char = '/';
// The method of HTTPS requests, whose URL is the host and port
const TUNNEL_METHOD: &str = "CONNECT";
const HTTP_PORT: u16 = 80;
const HTTPS_PORT: u16 = 443;
// Follows the log's lines containing $3, the last $1 up to its current size first, so none repeats or goes missing.
// -F survives log rotation.
const FOLLOW_MATCHING: &str = r#"lines=$1 log=$2 text=$3
size=$(wc -c < "$log") || exit
head -c "$size" "$log" | grep -F -- "$text" | tail -n "$lines"
tail -c +$((size + 1)) -F "$log" | grep --line-buffered -F -- "$text"
"#;
// Docker's default network, the proxy's way out
const EGRESS_NETWORK: &str = "bridge";

fn url() -> String {
    format!("http://{CONTAINER}:{PORT}")
}

/// Docker refuses or adds another `--network`, so a sandbox on another network sets `proxy: false`
pub fn run_args() -> Vec<String> {
    let url = url();
    let mut args = vec![format!("--network={NETWORK}")];
    for variable in PROXY_ENV {
        args.push(format!("--env={variable}={url}"));
    }
    for variable in PROXY_BYPASS_ENV {
        args.push(format!("--env={variable}={PROXY_BYPASS_HOSTS}"));
    }
    args
}

fn config_path() -> PathBuf {
    paths::state_dir().join("squid.conf")
}

/// `squid.conf` may be ahead of it when loading failed
fn loaded_config_path() -> PathBuf {
    paths::state_dir().join("squid.conf.loaded")
}

pub struct Proxy {
    pub image: String,
    pub hosts: Hosts,
}

pub enum Hosts {
    /// Except the proxy's own and link-local ones, like cloud metadata services
    All,
    Listed(Vec<String>),
}

/// The config file against what the proxy runs
#[derive(PartialEq)]
enum Config {
    Loaded,
    Changed,
    /// A proxy that mounted the previous file doesn't see it
    Created,
}

/// Starts the proxy and its network if needed, or updates its hosts
pub fn ensure(proxy: &Proxy) -> Result<()> {
    let content = config(&proxy.hosts);
    let config = write_config(&content)?;
    create_network()?;
    let state = start(&proxy.image, &config)?;
    connect(&state.networks)?;
    if config != Config::Loaded {
        paths::write_file(&loaded_config_path(), &content)?;
    }
    Ok(())
}

fn write_config(content: &str) -> Result<Config> {
    let path = config_path();
    // Docker leaves a directory at a bind mount's path if the file was gone when the container restarted
    if path.is_dir() {
        std::fs::remove_dir(&path)
            .with_context(|| format!("can't remove the directory {}", path.display()))?;
    }
    let current = std::fs::read_to_string(&path).ok();
    if current.as_deref() != Some(content) {
        // In place, as the bind mount keeps a replaced file
        paths::write_file(&path, content)?;
    }
    let loaded = std::fs::read_to_string(loaded_config_path()).ok();
    Ok(if current.is_none() {
        Config::Created
    } else if loaded.as_deref() == Some(content) {
        Config::Loaded
    } else {
        Config::Changed
    })
}

fn create_network() -> Result<()> {
    match network_internal(NETWORK) {
        Some(true) => Ok(()),
        // Not sndbx's to remove
        Some(false) => bail!(
            "the docker network {NETWORK} isn't internal, sandboxes on it could reach any host, \
             remove it with docker network rm {NETWORK} and run again"
        ),
        None if docker::run_muted(&["network", "create", "--internal", NETWORK]) => Ok(()),
        None => bail!("can't create the docker network {NETWORK}"),
    }
}

/// Keeps a running proxy and reloads its config, or replaces it
fn start(image: &str, config: &Config) -> Result<ContainerState> {
    match container_state(CONTAINER) {
        Some(state)
            if state.up
                && state.image == image
                && mounts_config(&state)
                && *config != Config::Created =>
        {
            reload(config)?;
            Ok(state)
        }
        // Stopped (its network may be gone), restarting, from another image, or mounting another or a
        // recreated config file
        state => replace(state.is_some(), image),
    }
}

fn reload(config: &Config) -> Result<()> {
    if *config != Config::Changed {
        return Ok(());
    }
    check_config(&["exec", CONTAINER, "squid", "-k", "parse"])?;
    if !docker::run_muted(&["exec", CONTAINER, "squid", "-k", "reconfigure"]) {
        bail!(start_error());
    }
    Ok(())
}

/// Runs a new proxy once squid accepts the config, removing the old one if `exists`
fn replace(exists: bool, image: &str) -> Result<ContainerState> {
    // Pulled first to show progress and docker's own error
    if !docker::image_exists(image) && !docker::run(&["pull", image]) {
        bail!("can't pull the proxy image {image}");
    }
    let volume = format!("{}:{CONFIG_TARGET}:ro", config_path().display());
    // Before removing the old proxy, so it keeps running if squid rejects the config
    check_config(&[
        "run",
        "--rm",
        "--entrypoint=squid",
        "-v",
        &volume,
        image,
        "-k",
        "parse",
    ])?;
    // --volumes removes the log and cache volumes, which would pile up
    if exists && !docker::run_muted(&["rm", "-f", "--volumes", CONTAINER]) {
        bail!("can't remove the proxy {CONTAINER} to replace it");
    }
    let restart = "--restart=unless-stopped";
    if !docker::run_muted(&[
        "run", "-d", "--name", CONTAINER, restart, "-v", &volume, image,
    ]) {
        bail!(start_error());
    }
    Ok(ContainerState {
        up: true,
        image: image.to_string(),
        networks: vec![EGRESS_NETWORK.to_string()],
        mounts: vec![config_path().display().to_string()],
    })
}

fn mounts_config(state: &ContainerState) -> bool {
    let path = config_path();
    state.mounts.iter().any(|mount| Path::new(mount) == path)
}

fn connect(networks: &[String]) -> Result<()> {
    for network in [EGRESS_NETWORK, NETWORK] {
        if !networks.iter().any(|name| name == network)
            && !docker::run_muted(&["network", "connect", network, CONTAINER])
        {
            bail!("can't connect the proxy {CONTAINER} to the docker network {network}");
        }
    }
    Ok(())
}

/// Shows only squid's errors, as it prints a line per directive, or all of docker's output if there are none
fn check_config(args: &[&str]) -> Result<()> {
    let Err(output) = docker::run_collecting_errors(args) else {
        return Ok(());
    };
    let errors: Vec<&str> = output
        .lines()
        .filter(|line| line.contains("| ERROR: ") || line.contains("| FATAL: "))
        .collect();
    if errors.is_empty() {
        eprint!("{output}");
    } else {
        eprintln!("{}", errors.join("\n"));
    }
    bail!(
        "squid can't use the proxy's config {}",
        config_path().display()
    );
}

pub struct LogOptions {
    pub lines: u64,
    /// Only the denied requests
    pub denied: bool,
    /// With sandbox names, local times and readable sizes
    pub pretty: bool,
}

pub fn follow_log(options: &LogOptions) -> Result<()> {
    if !container_state(CONTAINER).is_some_and(|state| state.up) {
        bail!("the proxy {CONTAINER} isn't running, sndbx run starts it");
    }
    let lines = options.lines.to_string();
    let denied = denied_text();
    let command: &[&str] = if options.denied {
        &[
            "sh",
            "-c",
            FOLLOW_MATCHING,
            "sh",
            &lines,
            ACCESS_LOG,
            &denied,
        ]
    } else {
        // -F survives log rotation
        &["tail", "-n", &lines, "-F", ACCESS_LOG]
    };
    if options.pretty {
        pretty::follow(CONTAINER, command, NETWORK)
    } else {
        docker::exec_until_interrupted(CONTAINER, command)
    }
}

/// In denied requests' lines
fn denied_text() -> String {
    format!(" {DENIED_RESULT}{STATUS_SEPARATOR}")
}

/// Time like sndbx's log, client, squid result/HTTP status, method, URL, duration in ms, bytes sent and received
fn log_format() -> String {
    format!(
        "%{{{LOG_TIME_FORMAT}}}tl %>a %Ss{STATUS_SEPARATOR}%03>Hs %rm %ru \
         {DURATION_FIELD}%tr{DURATION_UNIT} {SENT_FIELD}%>st {RECEIVED_FIELD}%<st"
    )
}

fn start_error() -> String {
    format!("can't start the proxy, see docker logs {CONTAINER}")
}

/// HTTP to 80 and HTTPS to 443 only, no cache. Logs to a file, as squid's `proxy` user can't open the container's
/// stdout. A line is written when the request ends, which can be minutes later for an HTTPS tunnel.
fn config(hosts: &Hosts) -> String {
    // Squid refuses an acl without values. -n skips matching an IP's reverse DNS name, which its owner chooses.
    let (acl, allow) = match hosts {
        // Built-in acls, matching the resolved address
        Hosts::All => (
            String::new(),
            "http_access deny to_localhost\nhttp_access deny to_linklocal\nhttp_access allow all\n",
        ),
        Hosts::Listed(hosts) if hosts.is_empty() => (String::new(), ""),
        Hosts::Listed(hosts) => (
            format!("acl allowed dstdomain -n {}\n", hosts.join(" ")),
            "http_access allow allowed\n",
        ),
    };
    let log_format = log_format();
    format!(
        "http_port {PORT}\n\
         acl SSL_ports port {HTTPS_PORT}\n\
         acl Safe_ports port {HTTP_PORT} {HTTPS_PORT}\n\
         {acl}\
         http_access deny !Safe_ports\n\
         http_access deny {TUNNEL_METHOD} !SSL_ports\n\
         {allow}\
         http_access deny all\n\
         cache deny all\n\
         logformat sndbx {log_format}\n\
         access_log {ACCESS_LOG} sndbx\n"
    )
}

#[cfg(test)]
mod tests;
