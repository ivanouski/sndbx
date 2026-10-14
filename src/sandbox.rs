use crate::docker::{IMAGE_LABEL, check_docker, docker_exec, image_exists};
use std::path::{Path, PathBuf};

use crate::paths::{expand_path, identical, join, normalize};
use crate::settings::Image;
use crate::{Result, error, paths};

pub const MOUNT_ROOT: &str = "/workspace";
const MAX_PORT: u64 = 65_535;

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

    pub fn call(&self) -> Result<()> {
        check_docker()?;
        self.check_image()?;
        let workdir = self.check_workdir()?;
        std::fs::create_dir_all(&self.image.config_dir)?;

        let mut args = vec![String::from("run")];
        if !self.keep {
            args.push(String::from("--rm"));
        }
        args.extend([
            String::from("-it"),
            format!("--label={IMAGE_LABEL}={}", self.image.name),
            format!("--workdir={workdir}"),
        ]);
        args.extend(self.volume_args());
        args.extend(self.ports.iter().flat_map(|port| [String::from("-p"), port.clone()]));
        args.extend(self.docker_args.iter().cloned());
        args.push(self.image.tag.clone());

        docker_exec(&args)
    }

    fn volume_args(&self) -> Vec<String> {
        self.volumes
            .iter()
            .map(|volume| {
                let mut parts = vec![volume.source.display().to_string(), volume.target.clone()];
                parts.extend(volume.options.clone());
                parts.join(":")
            })
            .chain([format!(
                "{}:{}",
                self.image.config_dir.display(),
                self.image.config_target
            )])
            .flat_map(|volume| [String::from("-v"), volume])
            .collect()
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
            .find(|target| workspace(target))
            .or(targets.last())
            .map(ToString::to_string)
    }

    fn check_image(&self) -> Result<()> {
        if image_exists(&self.image.tag) {
            return Ok(());
        }
        error(format!("docker image '{}' does not exist", self.image.tag))
    }

    fn check_workdir(&self) -> Result<&str> {
        let targets = self.directory_targets();
        if targets.is_empty() {
            return error("no directory volume to start in");
        }

        let workdir = self.workdir.as_deref().unwrap_or_default();
        if targets.contains(&workdir) {
            return Ok(workdir);
        }

        let names: Vec<&str> = targets.iter().map(|target| workdir_name(target)).collect();
        error(format!(
            "workdir '{}' is not a directory volume, expected one of: {}",
            workdir_name(workdir),
            names.join(", ")
        ))
    }
}

fn volumes(specs: &[String]) -> Result<Vec<Volume>> {
    let mut volumes = specs
        .iter()
        .map(|spec| parse_volume(spec))
        .collect::<Result<Vec<_>>>()?;
    if !volumes.iter().any(|volume| workspace(&volume.target)) {
        volumes.insert(0, parse_volume(&paths::current_dir().to_string_lossy())?);
    }

    let mut targets: Vec<(&str, usize)> = Vec::new();
    for volume in &volumes {
        match targets.iter_mut().find(|(target, _)| *target == volume.target) {
            Some((_, count)) => *count += 1,
            None => targets.push((&volume.target, 1)),
        }
    }
    if let Some((target, _)) = targets.iter().find(|(_, count)| *count > 1) {
        return error(format!("more than one volume is mounted at {target}"));
    }

    Ok(volumes)
}

/// Docker's -v syntax, SRC[:DST[:OPTS]], except that SRC is always a host path
/// and DST defaults to `MOUNT_ROOT/<basename>`.
fn parse_volume(spec: &str) -> Result<Volume> {
    let parts: Vec<&str> = spec.split(':').collect();
    if parts.len() > 3 || parts.iter().any(|part| part.is_empty()) {
        return error(format!("invalid volume '{spec}', expected SRC[:DST[:OPTS]]"));
    }

    let source = expand_path(parts[0], None);
    if !source.exists() {
        return error(format!("{} does not exist", source.display()));
    }
    if [&paths::home(), Path::new("/")]
        .iter()
        .any(|dir| identical(dir, &source))
    {
        return error(format!("refusing to mount {}", source.display()));
    }

    let target = if let Some(target) = parts.get(1) {
        if !target.starts_with('/') {
            return error(format!("invalid volume '{spec}', DST must be an absolute path"));
        }
        container_path(target)
    } else {
        if !source.is_dir() {
            return error(format!("{} is not a directory", source.display()));
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

fn workspace(target: &str) -> bool {
    target == MOUNT_ROOT || target.starts_with(&format!("{MOUNT_ROOT}/"))
}

/// Docker's `-p` syntax, `[HOST_IP:]HOST:CONTAINER[/PROTOCOL]`, where `PORT` means `PORT:PORT`
fn normalize_port(port: &str) -> Result<String> {
    let (numbers, protocol) = match port.split_once('/') {
        Some((numbers, protocol @ ("tcp" | "udp"))) => (numbers, Some(protocol)),
        Some(_) => (port, Some("")),
        None => (port, None),
    };
    let parts: Vec<&str> = numbers.split(':').collect();

    // The numbers are digits; a host IP in front is digits and dots
    let digits = |part: &&str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    let address = |part: &&str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit() || b == b'.');
    let valid = protocol != Some("")
        && match parts.as_slice() {
            [port] => digits(port),
            [host, container] => address(host) && digits(container),
            [address_part, host, container] => address(address_part) && digits(host) && digits(container),
            _ => false,
        };
    if !valid {
        return error(format!("invalid port '{port}', expected PORT or HOST:CONTAINER"));
    }
    if port_numbers(&parts).iter().any(|&number| number > MAX_PORT) {
        return error(format!("invalid port '{port}', ports go up to {MAX_PORT}"));
    }

    Ok(match (parts.as_slice(), protocol) {
        ([bare], Some(protocol)) => format!("{bare}:{bare}/{protocol}"),
        ([bare], None) => format!("{bare}:{bare}"),
        _ => port.to_string(),
    })
}

/// The host and container ports, or the leading number of a host IP in front of one port
fn port_numbers(parts: &[&str]) -> Vec<u64> {
    parts[parts.len().saturating_sub(2)..]
        .iter()
        .map(|part| {
            part.bytes()
                .take_while(u8::is_ascii_digit)
                .fold(0_u64, |number, digit| {
                    number.saturating_mul(10).saturating_add(u64::from(digit - b'0'))
                })
        })
        .collect()
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
        ] {
            assert!(normalize_port(invalid).is_err(), "{invalid}");
        }
    }
}
