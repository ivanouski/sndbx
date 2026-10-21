use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, bail};

use crate::paths::shell_join;
use crate::{Result, paths};

const DOCKER: &str = "docker";
pub const IMAGE_LABEL: &str = "sndbx.image";

pub fn log_path() -> PathBuf {
    paths::state_dir().join("sndbx.log")
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

/// The user the image runs as, like `ubuntu` or `1000:1000`, empty for root, `None` if the image doesn't exist
pub fn image_user(tag: &str) -> Option<String> {
    let output = Command::new(DOCKER)
        .args(["image", "inspect", "--format", "{{.Config.User}}", tag])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Whether the network is internal, without a route out, `None` if it doesn't exist
pub fn network_internal(name: &str) -> Option<bool> {
    let output = Command::new(DOCKER)
        .args(["network", "inspect", "--format", "{{.Internal}}", name])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim() == "true")
}

pub struct ContainerState {
    /// Running and not waiting to restart after it exited
    pub up: bool,
    /// How often docker restarted it after it exited
    pub restarts: u64,
    /// The image as `docker run` was given it
    pub image: String,
    pub networks: Vec<String>,
    /// The host paths mounted in it
    pub mounts: Vec<String>,
}

/// A line each, as paths may have spaces: the state, the image, the networks, then a line per mount
pub const CONTAINER_STATE_FORMAT: &str = "{{.State.Running}}\n{{.State.Restarting}}\n{{.RestartCount}}\n\
                                          {{.Config.Image}}\n\
                                          {{range $name, $network := .NetworkSettings.Networks}}{{$name}} {{end}}\n\
                                          {{range .Mounts}}{{.Source}}\n{{end}}";

/// The container's state, `None` if it doesn't exist
pub fn container_state(name: &str) -> Option<ContainerState> {
    let output = Command::new(DOCKER)
        .args(["container", "inspect", "--format", CONTAINER_STATE_FORMAT, name])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let output = String::from_utf8_lossy(&output.stdout);
    let mut lines = output.lines();
    let running = lines.next() == Some("true");
    let restarting = lines.next() == Some("true");
    let restarts = lines.next()?.parse().ok()?;
    let image = lines.next()?.to_string();
    let networks = lines.next()?.split_whitespace().map(ToString::to_string).collect();
    Some(ContainerState {
        up: running && !restarting,
        restarts,
        image,
        networks,
        mounts: lines.filter(|line| !line.is_empty()).map(ToString::to_string).collect(),
    })
}

pub fn running_containers(image: &str) -> Result<Vec<String>> {
    containers(&["--filter", &label_filter(image)])
}

pub fn stopped_containers(image: &str) -> Result<Vec<String>> {
    containers(&["--all", "--filter", "status=exited", "--filter", &label_filter(image)])
}

/// The containers in any state that use the image, sndbx's sandboxes or not, which keep docker from removing it
pub fn image_containers(tag: &str) -> Result<Vec<String>> {
    containers(&["--all", "--filter", &format!("ancestor={tag}")])
}

fn label_filter(image: &str) -> String {
    format!("label={IMAGE_LABEL}={image}")
}

fn containers(ps_args: &[&str]) -> Result<Vec<String>> {
    let output = Command::new(DOCKER)
        .arg("ps")
        .args(ps_args)
        .args(["--format", "{{.Names}}"])
        .stderr(Stdio::null())
        .output();

    match output {
        Ok(output) if output.status.success() => Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(ToString::to_string)
            .collect()),
        _ => bail!("failed to list containers"),
    }
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

/// Replaces sndbx with docker, returning only if docker can't run
pub fn exec<S: AsRef<str>>(args: &[S]) -> Result<()> {
    log_command(args);
    let _ = std::io::stdout().flush();
    let e = Command::new(DOCKER).args(args.iter().map(AsRef::as_ref)).exec();
    Err(e).context(format!("can't run {DOCKER}"))
}

/// Runs docker and waits for it, whether it succeeded
pub fn run<S: AsRef<str>>(args: &[S]) -> bool {
    run_with_stdout(args, Stdio::inherit())
}

/// Runs docker like `run` without its output, like the ID `docker run -d` prints; its errors still show
pub fn run_muted<S: AsRef<str>>(args: &[S]) -> bool {
    run_with_stdout(args, Stdio::null())
}

fn run_with_stdout<S: AsRef<str>>(args: &[S], stdout: Stdio) -> bool {
    log_command(args);
    let _ = std::io::stdout().flush();
    Command::new(DOCKER)
        .args(args.iter().map(AsRef::as_ref))
        .stdout(stdout)
        .status()
        .is_ok_and(|status| status.success())
}

/// A log that can't be written doesn't stop the command
fn log_command<S: AsRef<str>>(args: &[S]) {
    let path = log_path();
    let command: Vec<&str> = std::iter::once(DOCKER).chain(args.iter().map(AsRef::as_ref)).collect();
    let line = format!("{} {}\n", jiff::Zoned::now().strftime("%FT%T%:z"), shell_join(&command));

    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| OpenOptions::new().create(true).append(true).open(&path))
        .and_then(|mut file| file.write_all(line.as_bytes()));
    if let Err(e) = written {
        eprintln!("Warning: can't write {}: {e}", path.display());
    }
}

/// Runs docker without its output: whether it succeeded, `None` if it can't run at all
fn run_quietly(args: &[&str]) -> Option<bool> {
    Command::new(DOCKER)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()
        .map(|status| status.success())
}
