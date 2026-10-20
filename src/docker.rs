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
    paths::xdg_dir("XDG_STATE_HOME", ".local/state").join("sndbx/sndbx.log")
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
    log_command(args);
    let _ = std::io::stdout().flush();
    Command::new(DOCKER)
        .args(args.iter().map(AsRef::as_ref))
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
