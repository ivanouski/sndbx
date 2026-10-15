use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail};

use crate::docker::{self, IMAGE_LABEL, check_docker, image_exists};
use crate::paths::{expand_path, is_same_file, join, normalize};
use crate::settings::Image;
use crate::{Result, paths};

pub const MOUNT_ROOT: &str = "/workspace";

pub struct Options {
    pub volumes: Vec<String>,
    pub ports: Vec<String>,
    pub workdir: Option<String>,
    pub keep: bool,
    pub docker_args: Vec<String>,
}

struct Volume {
    source: PathBuf,
    target: String,
    options: Option<String>,
}

pub struct Sandbox {
    image: Image,
    keep: bool,
    docker_args: Vec<String>,
    volumes: Vec<Volume>,
    ports: Vec<String>,
    workdir: Option<String>,
}

impl Sandbox {
    pub fn new(image: Image, options: Options) -> Result<Self> {
        let mut docker_args = image.docker_args.clone();
        docker_args.extend(options.docker_args);

        let volumes = volumes(&options.volumes)?;

        let mut ports: Vec<String> = Vec::new();
        for port in image.ports.iter().chain(&options.ports) {
            let port = normalize_port(port)?;
            if !ports.contains(&port) {
                ports.push(port);
            }
        }

        let mut sandbox = Self {
            image,
            keep: options.keep,
            docker_args,
            volumes,
            ports,
            workdir: None,
        };
        sandbox.workdir = match options.workdir {
            Some(name) => Some(workdir_target(&name)),
            None => sandbox.default_workdir(),
        };
        Ok(sandbox)
    }

    pub fn run(&self) -> Result<()> {
        check_docker()?;
        self.check_image()?;
        let workdir = self.check_workdir()?;
        std::fs::create_dir_all(&self.image.config_dir)?;

        let label = format!("--label={IMAGE_LABEL}={}", self.image.name);
        let workdir = format!("--workdir={workdir}");
        let volumes = self.volume_specs();

        let mut args = vec!["run"];
        if !self.keep {
            args.push("--rm");
        }
        args.extend(["-it", &label, &workdir]);
        for volume in &volumes {
            args.extend(["-v", volume]);
        }
        for port in &self.ports {
            args.extend(["-p", port]);
        }
        args.extend(self.docker_args.iter().map(String::as_str));
        args.push(&self.image.tag);

        docker::exec(&args)
    }

    /// Each volume as docker's `SRC:DST[:OPTS]`, the image's config directory last
    fn volume_specs(&self) -> Vec<String> {
        let mut specs: Vec<String> = self
            .volumes
            .iter()
            .map(|volume| match &volume.options {
                Some(options) => format!("{}:{}:{options}", volume.source.display(), volume.target),
                None => format!("{}:{}", volume.source.display(), volume.target),
            })
            .collect();
        specs.push(format!(
            "{}:{}",
            self.image.config_dir.display(),
            self.image.config_target
        ));
        specs
    }

    fn directory_targets(&self) -> Vec<&str> {
        self.volumes
            .iter()
            .filter(|volume| volume.source.is_dir())
            .map(|volume| volume.target.as_str())
            .collect()
    }

    fn default_workdir(&self) -> Option<String> {
        let targets = self.directory_targets();
        targets
            .iter()
            .rev()
            .find(|target| is_in_workspace(target))
            .or(targets.last())
            .map(ToString::to_string)
    }

    fn check_image(&self) -> Result<()> {
        if image_exists(&self.image.tag) {
            return Ok(());
        }
        bail!("docker image '{}' does not exist", self.image.tag)
    }

    fn check_workdir(&self) -> Result<&str> {
        let targets = self.directory_targets();
        if targets.is_empty() {
            bail!("no directory volume to start in");
        }

        let workdir = self.workdir.as_deref().unwrap_or_default();
        if targets.contains(&workdir) {
            return Ok(workdir);
        }

        let names: Vec<&str> = targets.iter().map(|target| workdir_name(target)).collect();
        bail!(
            "workdir '{}' is not a directory volume, expected one of: {}",
            workdir_name(workdir),
            names.join(", ")
        )
    }
}

fn volumes(specs: &[String]) -> Result<Vec<Volume>> {
    let mut volumes = specs
        .iter()
        .map(|spec| parse_volume(spec))
        .collect::<Result<Vec<_>>>()?;
    if !volumes.iter().any(|volume| is_in_workspace(&volume.target)) {
        volumes.insert(0, parse_volume(&paths::current_dir().to_string_lossy())?);
    }

    let mut targets = HashSet::new();
    if let Some(volume) = volumes.iter().find(|volume| !targets.insert(&volume.target)) {
        bail!("more than one volume is mounted at {}", volume.target);
    }

    Ok(volumes)
}

/// Docker's -v syntax, SRC[:DST[:OPTS]], except that SRC is always a host path
/// and DST defaults to `MOUNT_ROOT/<basename>`.
fn parse_volume(spec: &str) -> Result<Volume> {
    let parts: Vec<&str> = spec.split(':').collect();
    if parts.len() > 3 || parts.iter().any(|part| part.is_empty()) {
        bail!("invalid volume '{spec}', expected SRC[:DST[:OPTS]]");
    }

    let source = expand_path(parts[0], None);
    if !source.exists() {
        bail!("{} does not exist", source.display());
    }
    if [&paths::home(), Path::new("/")]
        .iter()
        .any(|dir| is_same_file(dir, &source))
    {
        bail!("refusing to mount {}", source.display());
    }

    let target = if let Some(target) = parts.get(1) {
        if !target.starts_with('/') {
            bail!("invalid volume '{spec}', DST must be an absolute path");
        }
        container_path(target)
    } else {
        if !source.is_dir() {
            bail!("{} is not a directory", source.display());
        }
        // Not the root, it's refused above
        mount_target(&source.file_name().unwrap_or_default().to_string_lossy())
    };

    Ok(Volume {
        source,
        target,
        options: parts.get(2).map(ToString::to_string),
    })
}

fn workdir_target(name: &str) -> String {
    if name.starts_with('/') {
        container_path(name)
    } else {
        mount_target(name)
    }
}

fn workdir_name(target: &str) -> &str {
    target.strip_prefix(&format!("{MOUNT_ROOT}/")).unwrap_or(target)
}

fn is_in_workspace(target: &str) -> bool {
    target == MOUNT_ROOT || target.starts_with(&format!("{MOUNT_ROOT}/"))
}

/// Docker's `-p` syntax, `[HOST_IP:]HOST:CONTAINER[/PROTOCOL]`, where `PORT` means `PORT:PORT`
fn normalize_port(spec: &str) -> Result<String> {
    let invalid = || anyhow!("invalid port '{spec}', expected PORT or HOST:CONTAINER");

    let (numbers, protocol) = match spec.split_once('/') {
        None => (spec, None),
        Some((numbers, protocol @ ("tcp" | "udp"))) => (numbers, Some(protocol)),
        Some(_) => return Err(invalid()),
    };

    // Two fields are HOST:CONTAINER, or HOST_IP:CONTAINER when the first has dots
    let fields: Vec<&str> = numbers.split(':').collect();
    let (host_ip, ports) = match fields.as_slice() {
        [ip, _] if ip.contains('.') => (Some(*ip), &fields[1..]),
        [_] | [_, _] => (None, &fields[..]),
        [ip, _, _] => (Some(*ip), &fields[1..]),
        _ => return Err(invalid()),
    };
    if host_ip.is_some_and(|ip| !is_ip_address(ip)) || !ports.iter().all(|port| is_digits(port)) {
        return Err(invalid());
    }
    if ports.iter().any(|port| port.parse::<u16>().is_err()) {
        bail!("invalid port '{spec}', ports go up to {}", u16::MAX);
    }

    Ok(match (fields.as_slice(), protocol) {
        ([port], Some(protocol)) => format!("{port}:{port}/{protocol}"),
        ([port], None) => format!("{port}:{port}"),
        _ => spec.to_string(),
    })
}

fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// Digits and dots, docker checks the rest
fn is_ip_address(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit() || b == b'.')
}

fn mount_target(name: &str) -> String {
    join(&[MOUNT_ROOT, name])
}

/// An absolute path in the container without `.`, `..` or repeated slashes
fn container_path(path: &str) -> String {
    normalize(Path::new(path)).to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_ports() {
        assert_eq!(normalize_port("3000").unwrap(), "3000:3000");
        assert_eq!(normalize_port("53/udp").unwrap(), "53:53/udp");
        assert_eq!(normalize_port("8080:80").unwrap(), "8080:80");
        assert_eq!(normalize_port("127.0.0.1:3080:3080").unwrap(), "127.0.0.1:3080:3080");
        assert!(normalize_port("http").is_err());
        assert!(normalize_port("8080:70000").is_err());
        assert!(normalize_port("99999999999999999999999:80").is_err());
        assert!(normalize_port("127.0.0.1:3080:70000").is_err());
        assert_eq!(normalize_port("1.2.3.4:80").unwrap(), "1.2.3.4:80");
        for invalid in [
            "",
            ":80",
            "80:",
            "1:2:3:4",
            "80/sctp",
            "80/",
            "a:80",
            "80:1.2",
            "1.2:3.4:5",
            "80/tcp/udp",
            "+80",
            "1.2.3.4:+80",
        ] {
            assert!(normalize_port(invalid).is_err(), "{invalid}");
        }
    }
}
