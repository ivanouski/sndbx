//! The proxy that proxied sandboxes reach the network through: they are on an internal network
//! with no route out, and the proxy, also on the default one, lets through only the allowed hosts, or every one

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};

use crate::docker::{self, ContainerState, container_state, network_internal};
use crate::{Result, paths};

const NETWORK: &str = "sndbx-internal";
const CONTAINER: &str = "sndbx-proxy";
const PORT: u16 = 3128;
const CONFIG_TARGET: &str = "/etc/squid/squid.conf";
const ACCESS_LOG: &str = "/var/log/squid/access.log";
// A line each in the access log with the time like sndbx's log, the sandbox's address, how squid handled the request
// and the HTTP status, the method and the URL, then how long it took in milliseconds and the bytes the sandbox sent
// and got
const LOG_FORMAT: &str = "%{%FT%T%z}tl %>a %Ss/%03>Hs %rm %ru dur=%trms in=%>st out=%<st";
// In a denied request's line, after the time and the sandbox's address
const DENIED: &str = " TCP_DENIED/";
// Follows a log's lines with a text from its last ones that have it: those up to the log's size now, then the ones
// after it, so none shows twice or goes missing. -F keeps following when the log is rotated or recreated.
const FOLLOW_MATCHING: &str = r#"lines=$1 log=$2 text=$3
size=$(wc -c < "$log") || exit
head -c "$size" "$log" | grep -F -- "$text" | tail -n "$lines"
tail -c +$((size + 1)) -F "$log" | grep --line-buffered -F -- "$text"
"#;
// Docker's default network, which the proxy reaches the hosts through
const EGRESS_NETWORK: &str = "bridge";
// Squid stops within a moment of starting when it can't run, like with a config it can't use
const STARTUP_CHECK_DELAY: Duration = Duration::from_secs(1);

/// The proxy's URL in the internal network
fn url() -> String {
    format!("http://{CONTAINER}:{PORT}")
}

/// The `docker run` arguments that put a sandbox behind the proxy: the internal network, and the proxy's
/// address for the tools that read it. A `--network` among the sandbox's docker args doesn't replace the
/// internal network, docker refuses it or adds it, so a sandbox on another network sets `proxy: false`.
pub fn run_args() -> Vec<String> {
    let url = url();
    let mut args = vec![format!("--network={NETWORK}")];
    for variable in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
        args.push(format!("--env={variable}={url}"));
    }
    for variable in ["NO_PROXY", "no_proxy"] {
        args.push(format!("--env={variable}=localhost,127.0.0.1,::1"));
    }
    args
}

fn config_path() -> PathBuf {
    paths::state_dir().join("squid.conf")
}

/// The config the proxy last loaded, which `squid.conf` may be ahead of when loading it failed
fn loaded_config_path() -> PathBuf {
    paths::state_dir().join("squid.conf.loaded")
}

/// The proxy a sandbox runs behind
pub struct Proxy {
    pub image: String,
    pub hosts: Hosts,
}

/// The hosts the proxy lets through
pub enum Hosts {
    /// Every one but the proxy's own and link-local ones, like a cloud's metadata service
    All,
    /// Only these, none when it's empty
    Listed(Vec<String>),
}

/// How squid's config file compares to what the proxy runs with
#[derive(PartialEq)]
enum Config {
    Loaded,
    Changed,
    /// The file is new, so a proxy that mounted the one before it doesn't see it
    Created,
}

/// Starts the proxy and its network unless they're there, or updates its hosts
pub fn ensure(proxy: &Proxy) -> Result<()> {
    let content = config(&proxy.hosts);
    let config = write_config(&content)?;
    create_network()?;
    let (state, loaded) = start(&proxy.image, &config)?;
    connect(&state.networks)?;
    if loaded {
        check_up(state.restarts)?;
    }
    if config != Config::Loaded {
        paths::write_file(&loaded_config_path(), &content)?;
    }
    Ok(())
}

/// Writes squid's config unless the file has it already
fn write_config(content: &str) -> Result<Config> {
    let path = config_path();
    // Docker leaves an empty directory at a bind mount's path when the file is gone as the container restarts
    if path.is_dir() {
        std::fs::remove_dir(&path).with_context(|| format!("can't remove the directory {}", path.display()))?;
    }
    let current = std::fs::read_to_string(&path).ok();
    if current.as_deref() != Some(content) {
        // Written in place, the proxy's bind mount keeps the old file when it's replaced
        paths::write_file(&path, content)?;
    }
    Ok(if current.is_none() {
        Config::Created
    } else if std::fs::read_to_string(loaded_config_path()).ok().as_deref() == Some(content) {
        Config::Loaded
    } else {
        Config::Changed
    })
}

fn create_network() -> Result<()> {
    match network_internal(NETWORK) {
        Some(true) => Ok(()),
        // Sandboxes on it would reach any host, not just through the proxy. It isn't sndbx's to remove.
        Some(false) => bail!(
            "the docker network {NETWORK} isn't internal, sandboxes on it could reach any host, \
             remove it with docker network rm {NETWORK} and run again"
        ),
        None if docker::run_muted(&["network", "create", "--internal", NETWORK]) => Ok(()),
        None => bail!("can't create the docker network {NETWORK}"),
    }
}

/// Runs the proxy from `image` unless it's up with it, or reloads its config when it changed.
/// Its state, and whether it loaded a config, which `check_up` then checks it survived.
fn start(image: &str, config: &Config) -> Result<(ContainerState, bool)> {
    match container_state(CONTAINER) {
        Some(state) if state.up && state.image == image && mounts_config(&state) && *config != Config::Created => {
            Ok((state, reload(config)?))
        }
        // A stopped proxy may be on a network that was removed since, which keeps it from starting,
        // one that keeps restarting doesn't run, one from another image isn't the configured one,
        // and one that mounts another config file, like from another XDG_STATE_HOME, or one that's gone
        // can't load the new one, so they're all replaced
        state => Ok((replace(state.is_some(), image)?, true)),
    }
}

/// Has the running proxy reload its config when it changed, whether it did
fn reload(config: &Config) -> Result<bool> {
    let changed = *config == Config::Changed;
    if changed && !docker::run_muted(&["exec", CONTAINER, "squid", "-k", "reconfigure"]) {
        bail!(start_error());
    }
    Ok(changed)
}

/// Runs the proxy from `image`, after removing the one there is when `exists`, its state as it starts
fn replace(exists: bool, image: &str) -> Result<ContainerState> {
    // With its anonymous volumes, the squid image's log and cache, which would pile up otherwise
    if exists && !docker::run_muted(&["rm", "-f", "-v", CONTAINER]) {
        bail!("can't remove the proxy {CONTAINER} to replace it");
    }
    let volume = format!("{}:{CONFIG_TARGET}:ro", config_path().display());
    let restart = "--restart=unless-stopped";
    if !docker::run_muted(&["run", "-d", "--name", CONTAINER, restart, "-v", &volume, image]) {
        bail!(start_error());
    }
    Ok(ContainerState {
        up: true,
        restarts: 0,
        image: image.to_string(),
        networks: vec![EGRESS_NETWORK.to_string()],
        mounts: vec![config_path().display().to_string()],
    })
}

fn mounts_config(state: &ContainerState) -> bool {
    let path = config_path();
    state.mounts.iter().any(|mount| Path::new(mount) == path)
}

/// Connects the proxy to the networks it isn't on yet, also when connecting it failed before
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

/// Checks that squid still runs a moment after loading its config, not restarted since `restarts`
fn check_up(restarts: u64) -> Result<()> {
    std::thread::sleep(STARTUP_CHECK_DELAY);
    if !container_state(CONTAINER).is_some_and(|state| state.up && state.restarts == restarts) {
        bail!(start_error());
    }
    Ok(())
}

/// Follows the running proxy's access log from its last `lines` lines, only the denied requests' with `denied`
pub fn follow_log(lines: u64, denied: bool) -> Result<()> {
    if !container_state(CONTAINER).is_some_and(|state| state.up) {
        bail!("the proxy {CONTAINER} isn't running, sndbx run starts it");
    }
    let lines = lines.to_string();
    let command: &[&str] = if denied {
        &["sh", "-c", FOLLOW_MATCHING, "sh", &lines, ACCESS_LOG, DENIED]
    } else {
        // -F keeps following when the log is rotated or recreated
        &["tail", "-n", &lines, "-F", ACCESS_LOG]
    };
    docker::exec_until_interrupted(CONTAINER, command)
}

fn start_error() -> String {
    format!("can't start the proxy, see docker logs {CONTAINER}")
}

/// Squid's config: HTTP to port 80 and HTTPS to port 443 only, only to the hosts, no cache. Requests go to squid's own
/// access log, its `proxy` user can't open the container's stdout, a line each in `LOG_FORMAT`. Squid writes the line
/// when the request ends, which for an HTTPS tunnel a client keeps open can be minutes after it started.
fn config(hosts: &Hosts) -> String {
    // Squid refuses an acl without values. With -n, squid matches only the host the request names, without it
    // the reverse DNS name of an IP too, which whoever runs the IP chooses, like x.github.com.
    let (acl, allow) = match hosts {
        // Squid's built-in acls, which match the address the host resolves to
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
    format!(
        "http_port {PORT}\n\
         acl SSL_ports port 443\n\
         acl Safe_ports port 80 443\n\
         {acl}\
         http_access deny !Safe_ports\n\
         http_access deny CONNECT !SSL_ports\n\
         {allow}\
         http_access deny all\n\
         cache deny all\n\
         logformat sndbx {LOG_FORMAT}\n\
         access_log {ACCESS_LOG} sndbx\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configs() {
        assert_eq!(
            config(&Hosts::Listed(vec![".openai.com".to_string(), "pypi.org".to_string()])),
            "http_port 3128\n\
             acl SSL_ports port 443\n\
             acl Safe_ports port 80 443\n\
             acl allowed dstdomain -n .openai.com pypi.org\n\
             http_access deny !Safe_ports\n\
             http_access deny CONNECT !SSL_ports\n\
             http_access allow allowed\n\
             http_access deny all\n\
             cache deny all\n\
             logformat sndbx %{%FT%T%z}tl %>a %Ss/%03>Hs %rm %ru dur=%trms in=%>st out=%<st\n\
             access_log /var/log/squid/access.log sndbx\n"
        );
    }

    #[test]
    fn denies_all_without_hosts() {
        let config = config(&Hosts::Listed(Vec::new()));
        assert!(!config.contains("allowed"), "{config}");
        assert!(config.contains("http_access deny all\n"), "{config}");
    }

    #[test]
    fn allows_every_host_on_the_ports() {
        assert_eq!(
            config(&Hosts::All),
            "http_port 3128\n\
             acl SSL_ports port 443\n\
             acl Safe_ports port 80 443\n\
             http_access deny !Safe_ports\n\
             http_access deny CONNECT !SSL_ports\n\
             http_access deny to_localhost\n\
             http_access deny to_linklocal\n\
             http_access allow all\n\
             http_access deny all\n\
             cache deny all\n\
             logformat sndbx %{%FT%T%z}tl %>a %Ss/%03>Hs %rm %ru dur=%trms in=%>st out=%<st\n\
             access_log /var/log/squid/access.log sndbx\n"
        );
    }

    #[test]
    fn follows_the_last_matching_lines_then_new_ones() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        use std::sync::mpsc;

        let log = std::env::temp_dir().join(format!("sndbx-follow-matching-{}.log", std::process::id()));
        std::fs::write(
            &log,
            "1 TCP_DENIED/403\n2 TCP_TUNNEL/200\n3 TCP_DENIED/403\n4 TCP_DENIED/403\n",
        )
        .unwrap();
        // In its own process group, so killing that stops tail and grep too
        let mut child = Command::new("sh")
            .args(["-c", FOLLOW_MATCHING, "sh", "2", log.to_str().unwrap(), DENIED])
            .stdout(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let stdout = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if sender.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        let next = || receiver.recv_timeout(Duration::from_secs(5)).unwrap_or_default();

        let earlier = [next(), next()];
        let mut file = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        file.write_all(b"5 TCP_TUNNEL/200\n6 TCP_DENIED/403\n").unwrap();
        let later = next();

        // A negative PID is the process group, which procps' kill takes only after a signal and --
        let killed = Command::new("kill")
            .args(["-TERM", "--", &format!("-{}", child.id())])
            .status()
            .unwrap();
        assert!(killed.success());
        child.wait().unwrap();
        std::fs::remove_file(&log).unwrap();
        assert_eq!(earlier, ["3 TCP_DENIED/403", "4 TCP_DENIED/403"]);
        assert_eq!(later, "6 TCP_DENIED/403");
    }
}
