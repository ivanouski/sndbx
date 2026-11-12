use std::fs::OpenOptions;
use std::io::{IsTerminal, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};

use anyhow::{Context, bail};

use crate::paths::shell_join;
use crate::{Result, paths};

mod pretty;

const DOCKER: &str = "docker";
const TAIL: &str = "tail";
pub const IMAGE_LABEL: &str = "sndbx.image";
// The command log's time, which `logs commands --pretty` parses back
const LOG_TIME_FORMAT: &str = "%FT%T%:z";

pub fn log_path() -> PathBuf {
    paths::state_dir().join("sndbx.log")
}

/// Follows the command log like `tail -f`, starting with the last `lines`, formatted if `pretty`
pub fn follow_log(lines: u64, pretty: bool) -> Result<()> {
    let path = log_path();
    if !path.exists() {
        bail!("no log yet at {}", path.display());
    }

    let mut tail = Command::new(TAIL);
    tail.args(["-n", &lines.to_string(), "-f"]).arg(&path);
    if !pretty {
        return Err(tail_error(tail.exec()));
    }
    let child = tail.stdout(Stdio::piped()).spawn().map_err(tail_error)?;
    pretty::follow(child)
}

fn tail_error(e: std::io::Error) -> anyhow::Error {
    if e.kind() == std::io::ErrorKind::NotFound {
        anyhow::anyhow!("{TAIL} is not installed")
    } else {
        anyhow::Error::new(e).context(format!("can't run {TAIL}"))
    }
}

pub fn check_docker() -> Result<()> {
    match run_quietly(&["info"]) {
        None => bail!("docker is not installed"),
        Some(false) => bail!("docker is not running"),
        Some(true) => Ok(()),
    }
}

pub fn image_exists(tag: &str) -> bool {
    run_quietly(&["image", "inspect", tag]) == Some(true)
}

/// Empty for root, `None` if the image doesn't exist
pub fn image_user(tag: &str) -> Option<String> {
    output(&["image", "inspect", "--format", "{{.Config.User}}", tag])
        .map(|user| user.trim().to_string())
}

/// A user's uid and gid
pub struct Ids {
    pub uid: u32,
    pub gid: u32,
}

/// The ids base is built with, the host user's on Linux, as bind mounts keep the host's owners there; `None` for
/// the image's own: on other hosts, whose docker maps the owners, for root, and when the daemon maps the users
/// itself, rootless or with userns-remap
pub fn host_ids() -> Option<Ids> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    // Owned by the process's effective user and group
    let process = std::fs::metadata("/proc/self").ok()?;
    if process.uid() == 0 {
        return None;
    }
    let security_options = output(&["info", "--format", "{{.SecurityOptions}}"])?;
    (!has_user_mapping(&security_options)).then(|| Ids {
        uid: process.uid(),
        gid: process.gid(),
    })
}

fn has_user_mapping(security_options: &str) -> bool {
    ["name=rootless", "name=userns"]
        .iter()
        .any(|option| security_options.contains(option))
}

/// `None` if the network doesn't exist
pub fn network_internal(name: &str) -> Option<bool> {
    output(&["network", "inspect", "--format", "{{.Internal}}", name])
        .map(|internal| internal.trim() == "true")
}

pub struct ContainerState {
    /// Running and not restarting
    pub up: bool,
    /// As given to `docker run`
    pub image: String,
    pub networks: Vec<String>,
    /// Host paths
    pub mounts: Vec<String>,
}

/// A value per line, as paths may have spaces
const CONTAINER_STATE_FORMAT: &str = "{{.State.Running}}\n{{.State.Restarting}}\n\
                                          {{.Config.Image}}\n\
                                          {{range $name, $network := .NetworkSettings.Networks}}{{$name}} {{end}}\n\
                                          {{range .Mounts}}{{.Source}}\n{{end}}";

/// `None` if the container doesn't exist
pub fn container_state(name: &str) -> Option<ContainerState> {
    let state = output(&[
        "container",
        "inspect",
        "--format",
        CONTAINER_STATE_FORMAT,
        name,
    ])?;
    let mut lines = state.lines();
    let running = lines.next() == Some("true");
    let restarting = lines.next() == Some("true");
    let image = lines.next()?.to_string();
    let networks = lines
        .next()?
        .split_whitespace()
        .map(ToString::to_string)
        .collect();
    Some(ContainerState {
        up: running && !restarting,
        image,
        networks,
        mounts: lines
            .filter(|line| !line.is_empty())
            .map(ToString::to_string)
            .collect(),
    })
}

/// A running container's address on a network
#[derive(Clone, Debug, PartialEq)]
pub struct NetworkContainer {
    pub name: String,
    pub ip: String,
    pub started: jiff::Timestamp,
}

/// The name, start time and a `NETWORK=IP` field per network, as names and addresses have no spaces
const NETWORK_CONTAINER_FORMAT: &str = "{{.Name}} {{.State.StartedAt}}\
                                        {{range $name, $network := .NetworkSettings.Networks}} \
                                        {{$name}}={{$network.IPAddress}}{{end}}";
/// Between a network and the container's address on it
const NETWORK_IP_SEPARATOR: char = '=';
/// Before the names `inspect` prints
const NAME_PREFIX: char = '/';

/// Empty if docker fails, leaving out containers that stop meanwhile
pub fn network_containers(network: &str) -> Vec<NetworkContainer> {
    let names = containers(&["--filter", &format!("network={network}")]).unwrap_or_default();
    if names.is_empty() {
        return Vec::new();
    }
    let mut args = vec!["container", "inspect", "--format", NETWORK_CONTAINER_FORMAT];
    args.extend(names.iter().map(String::as_str));
    // Docker fails when a container is gone, but still prints the others
    let Some(output) = output_even_if_failed(&args) else {
        return Vec::new();
    };
    output
        .lines()
        .filter_map(|line| parse_network_container(line, network))
        .collect()
}

/// A line of `NETWORK_CONTAINER_FORMAT`, `None` without an address on the network
fn parse_network_container(line: &str, network: &str) -> Option<NetworkContainer> {
    let mut fields = line.split_whitespace();
    let name = fields.next()?.trim_start_matches(NAME_PREFIX);
    let started = fields.next()?.parse().ok()?;
    let ip = fields
        .filter_map(|field| field.split_once(NETWORK_IP_SEPARATOR))
        .find(|(name, ip)| *name == network && !ip.is_empty())?
        .1;
    Some(NetworkContainer {
        name: name.to_string(),
        ip: ip.to_string(),
        started,
    })
}

pub fn running_containers(image: &str) -> Result<Vec<String>> {
    containers(&["--filter", &label_filter(image)])
}

pub fn stopped_containers(image: &str) -> Result<Vec<String>> {
    containers(&[
        "--all",
        "--filter",
        "status=exited",
        "--filter",
        &label_filter(image),
    ])
}

/// Every container, as docker's names are unique across them all
pub fn container_names() -> Result<Vec<String>> {
    containers(&["--all"])
}

/// Any containers using the image, which keep docker from removing it
pub fn image_containers(tag: &str) -> Result<Vec<String>> {
    containers(&["--all", "--filter", &format!("ancestor={tag}")])
}

fn label_filter(image: &str) -> String {
    format!("label={IMAGE_LABEL}={image}")
}

fn containers(ps_args: &[&str]) -> Result<Vec<String>> {
    let mut args = vec!["ps"];
    args.extend(ps_args);
    args.extend(["--format", "{{.Names}}"]);
    let Some(output) = output(&args) else {
        bail!("failed to list containers");
    };
    Ok(output.lines().map(ToString::to_string).collect())
}

pub fn check_container(container: &str, containers: &[String], description: &str) -> Result<()> {
    if containers.is_empty() {
        bail!("no {description}");
    }
    if containers.iter().any(|name| name == container) {
        return Ok(());
    }

    bail!(
        "container '{container}' is not a {description}, expected one of: {}",
        containers.join(", ")
    )
}

/// Returns only if docker can't run
pub fn exec<S: AsRef<str>>(args: &[S]) -> Result<()> {
    prepare(args);
    let e = docker_command(args).exec();
    Err(e).context(format!("can't run {DOCKER}"))
}

pub fn exec_until_interrupted(container: &str, command: &[&str]) -> Result<()> {
    exec(&exec_args(container, command))
}

/// Like `exec_until_interrupted`, with the command's output piped to sndbx. With -t it ends lines in \r\n, and
/// docker turns off the terminal's own \n to \r\n while it runs.
pub fn spawn_until_interrupted(container: &str, command: &[&str]) -> Result<Child> {
    let args = exec_args(container, command);
    prepare(&args);
    docker_command(&args)
        .stdout(Stdio::piped())
        .spawn()
        .context(format!("can't run {DOCKER}"))
}

/// Whether `exec_until_interrupted` and `spawn_until_interrupted` run the command with -t, which makes Ctrl-C stop
/// it in the container too, but would end piped lines in \r\n
pub fn exec_uses_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn exec_args<'a>(container: &'a str, command: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec!["exec"];
    if exec_uses_terminal() {
        args.push("-it");
    }
    args.push(container);
    args.extend(command);
    args
}

pub fn run<S: AsRef<str>>(args: &[S]) -> bool {
    run_with_stdout(args, Stdio::inherit())
}

/// Hides stdout, like the ID `docker run -d` prints, but not errors
pub fn run_muted<S: AsRef<str>>(args: &[S]) -> bool {
    run_with_stdout(args, Stdio::null())
}

/// `Err` with docker's stderr, empty if docker can't run
pub fn run_collecting_errors<S: AsRef<str>>(args: &[S]) -> std::result::Result<(), String> {
    prepare(args);
    match docker_command(args).stdin(Stdio::null()).output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(String::from_utf8_lossy(&output.stderr).into_owned()),
        Err(_) => Err(String::new()),
    }
}

fn docker_command<S: AsRef<str>>(args: &[S]) -> Command {
    let mut command = Command::new(DOCKER);
    command.args(args.iter().map(AsRef::as_ref));
    command
}

fn run_with_stdout<S: AsRef<str>>(args: &[S], stdout: Stdio) -> bool {
    prepare(args);
    docker_command(args)
        .stdout(stdout)
        .status()
        .is_ok_and(|status| status.success())
}

/// Logs the command and flushes stdout so sndbx's output comes before docker's
fn prepare<S: AsRef<str>>(args: &[S]) {
    log_command(args);
    let _ = std::io::stdout().flush();
}

/// Only warns when the log can't be written
fn log_command<S: AsRef<str>>(args: &[S]) {
    let path = log_path();
    let command: Vec<&str> = std::iter::once(DOCKER)
        .chain(args.iter().map(AsRef::as_ref))
        .collect();
    let line = format!(
        "{} {}\n",
        jiff::Zoned::now().strftime(LOG_TIME_FORMAT),
        shell_join(&command)
    );

    let written = paths::create_parent_dir(&path)
        .and_then(|()| OpenOptions::new().create(true).append(true).open(&path))
        .and_then(|mut file| file.write_all(line.as_bytes()));
    if let Err(e) = written {
        eprintln!("Warning: can't write {}: {e}", path.display());
    }
}

/// Stdout, `None` if docker failed or can't run
fn output(args: &[&str]) -> Option<String> {
    let output = quiet_output(args)?;
    output.status.success().then(|| stdout_text(&output))
}

/// Stdout even if docker failed, `None` if it can't run
fn output_even_if_failed(args: &[&str]) -> Option<String> {
    quiet_output(args).map(|output| stdout_text(&output))
}

/// Without stderr, `None` if docker can't run
fn quiet_output(args: &[&str]) -> Option<Output> {
    docker_command(args).stderr(Stdio::null()).output().ok()
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// `None` if docker can't run
fn run_quietly(args: &[&str]) -> Option<bool> {
    docker_command(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()
        .map(|status| status.success())
}

#[cfg(test)]
mod tests;
