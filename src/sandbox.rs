use std::collections::HashSet;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail};

use crate::docker::{self, IMAGE_LABEL, image_user};
use crate::paths::{expand_path, is_same_file, join, normalize};
use crate::proxy::{self, Proxy};
use crate::settings::{self, Image};
use crate::{Result, paths, unique};

pub const MOUNT_ROOT: &str = "/workspace";
const HOME_ROOT: &str = "/home";
const ROOT_HOME: &str = "/root";

pub struct Options {
    pub volumes: Vec<String>,
    pub ports: Vec<String>,
    pub workdir: Option<String>,
    pub keep: bool,
    pub docker_args: Vec<String>,
    /// `None` for the open network
    pub proxy: Option<Proxy>,
}

struct Volume {
    source: PathBuf,
    target: String,
    options: Option<String>,
}

impl Volume {
    /// Docker's `SRC:DST[:OPTS]`
    fn spec(&self) -> String {
        match &self.options {
            Some(options) => format!("{}:{}:{options}", self.source.display(), self.target),
            None => format!("{}:{}", self.source.display(), self.target),
        }
    }
}

pub struct Sandbox {
    image: Image,
    keep: bool,
    docker_args: Vec<String>,
    volumes: Vec<Volume>,
    ports: Vec<String>,
    workdir: String,
    proxy: Option<Proxy>,
}

impl Sandbox {
    pub fn new(image: Image, options: Options) -> Result<Self> {
        let mut docker_args = image.docker_args.clone();
        docker_args.extend(options.docker_args);

        let volumes = volumes(&options.volumes)?;
        let workdir = match options.workdir {
            Some(name) => Some(workdir_target(&name)),
            None => default_workdir(&volumes),
        };
        let workdir = check_workdir(&volumes, workdir.as_deref().unwrap_or_default())?;

        let ports = ports(&image, &options.ports, options.proxy.is_some())?;

        Ok(Self {
            image,
            keep: options.keep,
            docker_args,
            volumes,
            ports,
            workdir,
            proxy: options.proxy,
        })
    }

    pub fn image(&self) -> &Image {
        &self.image
    }

    /// Expects docker checked and the image built
    pub fn run(&self) -> Result<()> {
        let config_target = self.config_target()?;
        std::fs::create_dir_all(&self.image.config_dir)?;
        if let Some(proxy) = &self.proxy {
            proxy::ensure(proxy)?;
        }
        docker::exec(&self.run_args(&config_target))
    }

    fn run_args(&self, config_target: &str) -> Vec<String> {
        let mut args = vec!["run".to_string()];
        if !self.keep {
            args.push("--rm".to_string());
        }
        args.extend([
            "-it".to_string(),
            format!("--label={IMAGE_LABEL}={}", self.image.name),
            format!("--workdir={}", self.workdir),
        ]);
        for volume in self.volume_specs(config_target) {
            args.extend(["-v".to_string(), volume]);
        }
        for port in &self.ports {
            args.extend(["-p".to_string(), port.clone()]);
        }
        // Before the docker args, so those can override the variables
        if self.proxy.is_some() {
            args.extend(proxy::run_args());
        }
        args.extend(self.docker_args.iter().cloned());
        args.push(self.image.tag.clone());
        args
    }

    fn volume_specs(&self, config_target: &str) -> Vec<String> {
        let mut specs: Vec<String> = self.volumes.iter().map(Volume::spec).collect();
        specs.push(format!("{}:{config_target}", self.image.config_dir.display()));
        specs
    }

    /// In the home of the configured or the image's user; fails if the image doesn't exist
    fn config_target(&self) -> Result<String> {
        let Some(image_user) = image_user(&self.image.tag) else {
            bail!("docker image '{}' does not exist", self.image.tag);
        };
        let user = self.image.user.as_deref().unwrap_or(&image_user);
        let Some(home) = home(user) else {
            bail!(
                "can't tell the home of user '{user}' that image '{}' runs as, set user to its name in {}",
                self.image.name,
                settings::path().display()
            );
        };
        Ok(join(&[&home, &self.image.config_target]))
    }
}

/// The sandbox starts in a directory volume
fn check_workdir(volumes: &[Volume], workdir: &str) -> Result<String> {
    let targets = directory_targets(volumes);
    if targets.is_empty() {
        bail!("no directory volume to start in");
    }
    if targets.contains(&workdir) {
        return Ok(workdir.to_string());
    }

    let names: Vec<&str> = targets.iter().map(|target| workdir_name(target)).collect();
    bail!(
        "workdir '{}' is not a directory volume, expected one of: {}",
        workdir_name(workdir),
        names.join(", ")
    )
}

/// Normalized and deduplicated; a proxied sandbox can't publish any
fn ports(image: &Image, extra: &[String], proxied: bool) -> Result<Vec<String>> {
    let ports = unique(
        image
            .ports
            .iter()
            .chain(extra)
            .map(|port| normalize_port(port))
            .collect::<Result<Vec<_>>>()?,
    );
    if proxied && !ports.is_empty() {
        let publisher = if image.ports.is_empty() {
            "-p publishes ports".to_string()
        } else {
            format!("image '{}' publishes ports", image.name)
        };
        bail!(
            "{publisher}, which a sandbox behind the proxy can't, set proxy: false for image '{}' in {}",
            image.name,
            settings::path().display()
        );
    }
    Ok(ports)
}

/// `user` is like docker's `--user`; `None` for a uid
fn home(user: &str) -> Option<String> {
    match user.split_once(':').map_or(user, |(name, _group)| name) {
        // Empty is docker's default
        "" | "root" | "0" => Some(ROOT_HOME.to_string()),
        uid if is_digits(uid) => None,
        name => Some(join(&[HOME_ROOT, name])),
    }
}

fn directory_targets(volumes: &[Volume]) -> Vec<&str> {
    volumes
        .iter()
        .filter(|volume| volume.source.is_dir())
        .map(|volume| volume.target.as_str())
        .collect()
}

/// The last directory volume in the workspace, or the last one anywhere
fn default_workdir(volumes: &[Volume]) -> Option<String> {
    let targets = directory_targets(volumes);
    targets
        .iter()
        .rev()
        .find(|target| is_in_workspace(target))
        .or(targets.last())
        .map(ToString::to_string)
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

/// Like docker's `-v SRC[:DST[:OPTS]]`, but SRC is a host path and DST defaults to `MOUNT_ROOT/<basename>`
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
        // The root is refused above
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
    Path::new(target).starts_with(MOUNT_ROOT)
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
        [ip, ports @ ..] if fields.len() == 3 || ip.contains('.') => (Some(*ip), ports),
        ports => (None, ports),
    };
    if !matches!(ports.len(), 1 | 2)
        || host_ip.is_some_and(|ip| ip.parse::<Ipv4Addr>().is_err())
        || !ports.iter().all(|port| is_digits(port))
    {
        return Err(invalid());
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

fn mount_target(name: &str) -> String {
    join(&[MOUNT_ROOT, name])
}

fn container_path(path: &str) -> String {
    normalize(Path::new(path)).to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn homes() {
        assert_eq!(home("ubuntu").as_deref(), Some("/home/ubuntu"));
        assert_eq!(home("dev:staff").as_deref(), Some("/home/dev"));
        assert_eq!(home("dev:1000").as_deref(), Some("/home/dev"));
        for root in ["", "root", "0", "root:root", "0:0"] {
            assert_eq!(home(root).as_deref(), Some("/root"), "{root}");
        }
        for uid in ["1000", "1000:1000"] {
            assert_eq!(home(uid), None, "{uid}");
        }
    }

    fn sandbox(keep: bool, proxy: Option<Proxy>) -> Sandbox {
        Sandbox {
            image: Image {
                name: "dev".to_string(),
                tag: "sndbx-dev".to_string(),
                config_dir: PathBuf::from("/config/dev"),
                user: None,
                config_target: ".claude".to_string(),
                ports: Vec::new(),
                docker_args: Vec::new(),
                proxy: proxy.is_some(),
            },
            keep,
            docker_args: vec!["--env=FOO=bar".to_string()],
            volumes: vec![Volume {
                source: PathBuf::from("/src/app"),
                target: "/workspace/app".to_string(),
                options: Some("ro".to_string()),
            }],
            ports: vec!["3000:3000".to_string()],
            workdir: "/workspace/app".to_string(),
            proxy,
        }
    }

    #[test]
    fn run_args() {
        assert_eq!(
            sandbox(false, None).run_args("/home/ubuntu/.claude"),
            [
                "run",
                "--rm",
                "-it",
                &format!("--label={IMAGE_LABEL}=dev"),
                "--workdir=/workspace/app",
                "-v",
                "/src/app:/workspace/app:ro",
                "-v",
                "/config/dev:/home/ubuntu/.claude",
                "-p",
                "3000:3000",
                "--env=FOO=bar",
                "sndbx-dev",
            ]
        );
    }

    #[test]
    fn run_args_keep_the_container_and_put_the_proxy_before_the_docker_args() {
        let proxy = Proxy {
            image: "squid".to_string(),
            hosts: proxy::Hosts::All,
        };
        let args = sandbox(true, Some(proxy)).run_args("/home/ubuntu/.claude");
        assert!(!args.contains(&"--rm".to_string()));
        let proxy_args = proxy::run_args();
        let start = args.iter().position(|arg| *arg == proxy_args[0]).unwrap();
        assert_eq!(args[start..start + proxy_args.len()], proxy_args);
        assert_eq!(args[start + proxy_args.len()..], ["--env=FOO=bar", "sndbx-dev"]);
    }

    #[test]
    fn normalizes_ports() {
        assert_eq!(normalize_port("3000").unwrap(), "3000:3000");
        assert_eq!(normalize_port("53/udp").unwrap(), "53:53/udp");
        assert_eq!(normalize_port("8080:80").unwrap(), "8080:80");
        assert_eq!(normalize_port("127.0.0.1:3080:3080").unwrap(), "127.0.0.1:3080:3080");
        assert!(normalize_port("http").is_err());
        // Docker checks the range
        assert_eq!(normalize_port("8080:70000").unwrap(), "8080:70000");
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
            "1.2.3.4.5:80",
            "...:80",
            "1.2.3.4:5:6:7",
            "1.2.3.4",
        ] {
            assert!(normalize_port(invalid).is_err(), "{invalid}");
        }
    }
}
