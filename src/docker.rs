use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use crate::paths::{join, shell_join};
use crate::{Error, Result, error, paths};

const DOCKER: &str = "docker";
pub const IMAGE_LABEL: &str = "sndbx.image";

pub fn log_path() -> String {
    join(&[&paths::home(), ".local", "state", "sndbx", "sndbx.log"])
}

pub fn check_docker() -> Result<()> {
    match quiet_system(&["info"]) {
        None => error("docker is not installed"),
        Some(false) => error("docker is not running"),
        Some(true) => Ok(()),
    }
}

pub fn image_exists(tag: &str) -> bool {
    quiet_system(&["image", "inspect", tag]) == Some(true)
}

pub fn running_containers(image: &str) -> Result<Vec<String>> {
    containers(image, &[])
}

pub fn stopped_containers(image: &str) -> Result<Vec<String>> {
    containers(image, &["--all", "--filter", "status=exited"])
}

fn containers(image: &str, ps_args: &[&str]) -> Result<Vec<String>> {
    let label = format!("label={IMAGE_LABEL}={image}");
    let output = Command::new(DOCKER)
        .arg("ps")
        .args(ps_args)
        .args(["--filter", &label, "--format", "{{.Names}}"])
        .stderr(Stdio::null())
        .output();

    match output {
        Ok(output) if output.status.success() => Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(ToString::to_string)
            .collect()),
        _ => error("failed to list containers"),
    }
}

pub fn check_container(container: &str, containers: &[String], description: &str) -> Result<()> {
    if containers.is_empty() {
        return error(format!("no {description}"));
    }
    if containers.iter().any(|name| name == container) {
        return Ok(());
    }

    error(format!(
        "container '{container}' is not a {description}, expected one of: {}",
        containers.join(", ")
    ))
}

/// Replaces sndbx with docker, returning only if docker can't run
pub fn docker_exec(args: &[String]) -> Result<()> {
    log_command(args);
    let _ = std::io::stdout().flush();
    let e = Command::new(DOCKER).args(args).exec();
    error(format!("can't run {DOCKER}: {e}"))
}

pub fn docker_system(args: &[String]) -> bool {
    log_command(args);
    let _ = std::io::stdout().flush();
    Command::new(DOCKER)
        .args(args)
        .status()
        .is_ok_and(|status| status.success())
}

/// A log that can't be written doesn't stop the command
fn log_command(args: &[String]) {
    let path = log_path();
    let mut command = vec![DOCKER.to_string()];
    command.extend_from_slice(args);
    let line = format!("{} {}\n", jiff::Zoned::now().strftime("%FT%T%:z"), shell_join(&command));

    let written = std::path::Path::new(&path)
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| OpenOptions::new().create(true).append(true).open(&path))
        .and_then(|mut file| file.write_all(line.as_bytes()));
    if let Err(e) = written {
        eprintln!("Warning: can't write {path}: {e}");
    }
}

/// `None` if the command can't run, like Ruby's `system` returning nil
fn quiet_system(args: &[&str]) -> Option<bool> {
    Command::new(DOCKER)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()
        .map(|status| status.success())
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Message(e.to_string())
    }
}
