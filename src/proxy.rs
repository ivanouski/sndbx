//! The proxy that proxied sandboxes reach the network through: they are on an internal network
//! with no route out, and the proxy, also on the default one, lets through only the allowed hosts

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};

use crate::docker::{self, ContainerState, container_state, network_internal};
use crate::{Result, paths};

const NETWORK: &str = "sndbx-internal";
pub const CONTAINER: &str = "sndbx-proxy";
const PORT: u16 = 3128;
const CONFIG_TARGET: &str = "/etc/squid/squid.conf";
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

pub fn config_path() -> PathBuf {
    paths::state_dir().join("squid.conf")
}

/// The config the proxy last loaded, which `squid.conf` may be ahead of when loading it failed
fn loaded_config_path() -> PathBuf {
    paths::state_dir().join("squid.conf.loaded")
}

/// The proxy a sandbox runs behind
pub struct Proxy {
    pub image: String,
    pub allowed_hosts: Vec<String>,
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
    let content = config(&proxy.allowed_hosts);
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
            let changed = *config == Config::Changed;
            if changed && !docker::run_muted(&["exec", CONTAINER, "squid", "-k", "reconfigure"]) {
                bail!(start_error());
            }
            Ok((state, changed))
        }
        state => {
            // A stopped proxy may be on a network that was removed since, which keeps it from starting,
            // one that keeps restarting doesn't run, one from another image isn't the configured one,
            // and one that mounts another config file, like from another XDG_STATE_HOME, or one that's gone
            // can't load the new one, so they're all replaced
            // With its anonymous volumes, the squid image's log and cache, which would pile up otherwise
            if state.is_some() && !docker::run_muted(&["rm", "-f", "-v", CONTAINER]) {
                bail!("can't remove the proxy {CONTAINER} to replace it");
            }
            let volume = format!("{}:{CONFIG_TARGET}:ro", config_path().display());
            let restart = "--restart=unless-stopped";
            if !docker::run_muted(&["run", "-d", "--name", CONTAINER, restart, "-v", &volume, image]) {
                bail!(start_error());
            }
            let state = ContainerState {
                up: true,
                restarts: 0,
                image: image.to_string(),
                networks: vec![EGRESS_NETWORK.to_string()],
                mounts: vec![config_path().display().to_string()],
            };
            Ok((state, true))
        }
    }
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

fn start_error() -> String {
    format!("can't start the proxy, see docker logs {CONTAINER}")
}

/// Squid's config: HTTP to port 80 and HTTPS to port 443 only, only to the hosts, no cache. Requests go to squid's own
/// access log, its `proxy` user can't open the container's stdout.
fn config(hosts: &[String]) -> String {
    // Squid refuses an acl without values. With -n, squid matches only the host the request names, without it
    // the reverse DNS name of an IP too, which whoever runs the IP chooses, like x.github.com.
    let (acl, allow) = if hosts.is_empty() {
        (String::new(), "")
    } else {
        (
            format!("acl allowed dstdomain -n {}\n", hosts.join(" ")),
            "http_access allow allowed\n",
        )
    };
    format!(
        "http_port {PORT}\nacl SSL_ports port 443\nacl Safe_ports port 80 443\n{acl}\
         http_access deny !Safe_ports\nhttp_access deny CONNECT !SSL_ports\n{allow}\
         http_access deny all\ncache deny all\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configs() {
        assert_eq!(
            config(&[".openai.com".to_string(), "pypi.org".to_string()]),
            "http_port 3128\n\
             acl SSL_ports port 443\n\
             acl Safe_ports port 80 443\n\
             acl allowed dstdomain -n .openai.com pypi.org\n\
             http_access deny !Safe_ports\n\
             http_access deny CONNECT !SSL_ports\n\
             http_access allow allowed\n\
             http_access deny all\n\
             cache deny all\n"
        );
    }

    #[test]
    fn denies_all_without_hosts() {
        let config = config(&[]);
        assert!(!config.contains("allowed"), "{config}");
        assert!(config.contains("http_access deny all\n"), "{config}");
    }
}
