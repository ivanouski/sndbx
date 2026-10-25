use std::fs::OpenOptions;
use std::io::{IsTerminal, Write};
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

/// Empty for root, `None` if the image doesn't exist
pub fn image_user(tag: &str) -> Option<String> {
    output(&["image", "inspect", "--format", "{{.Config.User}}", tag])
        .map(|user| user.trim().to_string())
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
    // -t makes Ctrl-C stop the command in the container too, but would end piped lines in \r\n
    let mut args = vec!["exec"];
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        args.push("-it");
    }
    args.push(container);
    args.extend(command);
    exec(&args)
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
        jiff::Zoned::now().strftime("%FT%T%:z"),
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
    let output = docker_command(args).stderr(Stdio::null()).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
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
