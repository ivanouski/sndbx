//! Reads docker, the settings and the proxy's log, once or in threads that send what they read

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::Value;
use sndbx::builder::Builder;
use sndbx::docker::{self, IMAGE_LABEL};
use sndbx::proxy::{self, LogLine};
use sndbx::settings::Settings;

use crate::model::{ImageInfo, LOG_LINES, Mount, Sandbox, Snapshot, Stats, short_size};

/// What the threads send the app
pub enum Event {
    Snapshot(Snapshot),
    Stats(HashMap<String, Stats>),
    /// The log is read again from its start
    LogReset,
    Log(LogLine),
    /// A background action ended, with a message or an error
    Done(Result<String, String>),
}

/// Refreshes when docker reports a change, or every so often
const REFRESH_EVERY: Duration = Duration::from_secs(10);
const STATS_EVERY: Duration = Duration::from_secs(2);
/// Before following the log or watching events again
const RETRY_AFTER: Duration = Duration::from_secs(2);

/// The threads, and how the app talks to them
pub struct Sources {
    /// What the threads and background actions send
    pub events: Receiver<Event>,
    /// For background actions to send through
    pub sender: Sender<Event>,
    /// A message asks for a refresh
    pub refresh: Sender<()>,
    shared: Shared,
}

impl Sources {
    /// The sandboxes whose stats to read
    pub fn set_running(&self, names: Vec<String>) {
        *self.shared.running.lock().unwrap() = names;
    }

    /// Stats are read only while they're shown
    pub fn show_stats(&self, shown: bool) {
        self.shared.stats_shown.store(shown, Ordering::Relaxed);
    }

    /// Stops the docker processes still running, which would outlive the threads
    pub fn stop(&self) {
        for id in self.shared.children.lock().unwrap().drain(..) {
            if let Ok(pid) = i32::try_from(id) {
                // SAFETY: kill only sends a signal
                unsafe {
                    libc::kill(pid, libc::SIGTERM);
                }
            }
        }
    }
}

/// What the app shares with the threads
#[derive(Clone, Default)]
struct Shared {
    running: Arc<Mutex<Vec<String>>>,
    stats_shown: Arc<AtomicBool>,
    /// The docker processes the threads run
    children: Arc<Mutex<Vec<u32>>>,
}

impl Shared {
    /// Passes `each` the lines docker prints until it exits. Closes docker's stdin then, which ends a tail in the
    /// proxy.
    fn read_lines<S: AsRef<str>>(&self, args: &[S], mut each: impl FnMut(String)) {
        let Ok(mut child) = docker::command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        else {
            return;
        };
        let id = child.id();
        self.children.lock().unwrap().push(id);
        let stdout = child.stdout.take().expect("piped");
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            each(line);
        }
        drop(child.stdin.take());
        let _ = child.wait();
        self.children.lock().unwrap().retain(|other| *other != id);
    }
}

/// What each thread gets
#[derive(Clone)]
struct Worker {
    events: Sender<Event>,
    refresh: Sender<()>,
    shared: Shared,
}

impl Worker {
    /// Ignores that the app quit, the threads end with it
    fn send(&self, event: Event) {
        let _ = self.events.send(event);
    }
}

/// Starts the threads
pub fn start() -> Sources {
    let (sender, events) = mpsc::channel();
    let (refresh, requests) = mpsc::channel();
    let worker = Worker {
        events: sender,
        refresh,
        shared: Shared::default(),
    };
    spawn(&worker, move |worker| refresh_loop(worker, &requests));
    spawn(&worker, watch_events);
    spawn(&worker, stats_loop);
    spawn(&worker, follow_log);
    Sources {
        events,
        sender: worker.events,
        refresh: worker.refresh,
        shared: worker.shared,
    }
}

fn spawn(worker: &Worker, work: impl FnOnce(&Worker) + Send + 'static) {
    let worker = worker.clone();
    std::thread::spawn(move || {
        sndbx::work_in_background();
        work(&worker);
    });
}

/// Every so often, and when asked: once for a burst of requests
fn refresh_loop(worker: &Worker, requests: &Receiver<()>) {
    loop {
        worker.send(Event::Snapshot(snapshot()));
        let _ = requests.recv_timeout(REFRESH_EVERY);
        while requests.try_recv().is_ok() {}
    }
}

/// Asks for a refresh on every container and image change but the execs, like the one following the log
fn watch_events(worker: &Worker) {
    let args = [
        "events",
        "--filter",
        "type=container",
        "--filter",
        "type=image",
        "--format",
        "{{.Action}}",
    ];
    loop {
        worker.shared.read_lines(&args, |action| {
            if !action.starts_with("exec_") {
                let _ = worker.refresh.send(());
            }
        });
        thread::sleep(RETRY_AFTER);
    }
}

/// While they're shown
fn stats_loop(worker: &Worker) {
    loop {
        let running = worker.shared.running.lock().unwrap().clone();
        if worker.shared.stats_shown.load(Ordering::Relaxed) && !running.is_empty() {
            worker.send(Event::Stats(stats(&running)));
        }
        thread::sleep(STATS_EVERY);
    }
}

/// Follows the log while the proxy runs, from its last lines each time it starts
fn follow_log(worker: &Worker) {
    let args = proxy::log_args(LOG_LINES as u64, true);
    loop {
        if proxy::is_up() {
            worker.send(Event::LogReset);
            worker.shared.read_lines(&args, |line| {
                if let Some(line) = LogLine::parse(&line) {
                    worker.send(Event::Log(line));
                }
            });
        }
        thread::sleep(RETRY_AFTER);
    }
}

/// The log's last lines, without following it
pub fn log_lines(proxy_up: bool) -> Vec<LogLine> {
    if !proxy_up {
        return Vec::new();
    }
    docker::output(&proxy::log_args(LOG_LINES as u64, false))
        .unwrap_or_default()
        .lines()
        .filter_map(LogLine::parse)
        .collect()
}

pub fn snapshot() -> Snapshot {
    let (settings, settings_error) = match Settings::load_or_default() {
        Ok(settings) => (Some(settings), None),
        Err(e) => (None, Some(format!("{e:#}"))),
    };
    let buildable = settings
        .as_ref()
        .and_then(|settings| Builder::new(settings).image_names().ok())
        .unwrap_or_default();
    let (sandboxes, docker_error) = match sandboxes() {
        Ok(sandboxes) => (sandboxes, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    let docker_up = docker_error.is_none();
    Snapshot {
        docker_error,
        sandboxes,
        proxy_up: docker_up && proxy::is_up(),
        images: if docker_up { images() } else { HashMap::new() },
        settings,
        settings_error,
        buildable,
    }
}

/// Running ones first
fn sandboxes() -> Result<Vec<Sandbox>, String> {
    let ids = sandbox_ids()?;
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut sandboxes: Vec<Sandbox> = inspect(&ids).iter().filter_map(parse_sandbox).collect();
    sandboxes.sort_by_key(|sandbox| (!sandbox.running(), sandbox.name.clone()));
    Ok(sandboxes)
}

/// The containers sndbx ran, running or not
fn sandbox_ids() -> Result<Vec<String>, String> {
    let filter = format!("label={IMAGE_LABEL}");
    let output = docker_output(&["ps", "--all", "--filter", &filter, "--format", "{{.ID}}"])
        .map_err(|_| "docker is not installed".to_string())?;
    if !output.status.success() {
        return Err("docker is not running".to_string());
    }
    let ids = String::from_utf8_lossy(&output.stdout);
    Ok(ids.split_whitespace().map(String::from).collect())
}

/// A container can go between ps and inspect, which then prints the others and fails
fn inspect(ids: &[String]) -> Vec<Value> {
    let mut args = vec!["inspect"];
    args.extend(ids.iter().map(String::as_str));
    docker_output(&args)
        .ok()
        .and_then(|output| serde_json::from_slice(&output.stdout).ok())
        .unwrap_or_default()
}

/// A container from `docker inspect`, `None` if sndbx didn't run it
fn parse_sandbox(container: &Value) -> Option<Sandbox> {
    let image = container
        .pointer("/Config/Labels")?
        .get(IMAGE_LABEL)?
        .as_str()?
        .to_string();
    let networks = container
        .pointer("/NetworkSettings/Networks")
        .and_then(Value::as_object);
    let proxy_ip = networks.and_then(|networks| ip_address(networks.get(proxy::NETWORK)?));
    let any_ip = || networks?.values().find_map(ip_address);
    Some(Sandbox {
        name: text(container, "/Name").trim_start_matches('/').to_string(),
        image,
        state: text(container, "/State/Status"),
        started: text(container, "/State/StartedAt").parse().ok(),
        workdir: text(container, "/Config/WorkingDir"),
        mounts: mounts(container.get("Mounts")),
        // As run asked for them, which a stopped sandbox keeps too
        ports: ports(container.pointer("/HostConfig/PortBindings")),
        networks: networks
            .map(|networks| networks.keys().cloned().collect())
            .unwrap_or_default(),
        ip: proxy_ip.clone().or_else(any_ip),
        proxy_ip,
        pids_limit: container
            .pointer("/HostConfig/PidsLimit")
            .and_then(Value::as_u64)
            .filter(|limit| *limit > 0),
    })
}

/// The string at `pointer`, empty if there's none
fn text(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// A network's address, `None` while the container is stopped
fn ip_address(network: &Value) -> Option<String> {
    Some(text(network, "/IPAddress")).filter(|ip| !ip.is_empty())
}

fn mounts(mounts: Option<&Value>) -> Vec<Mount> {
    mounts
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|mount| Mount {
            source: text(mount, "/Source"),
            destination: text(mount, "/Destination"),
        })
        .collect()
}

/// Like docker's `-p`: `127.0.0.1:3080:3080`, or `8080:80` on every address
fn ports(bindings: Option<&Value>) -> Vec<String> {
    let mut ports = Vec::new();
    for (port, bindings) in bindings.and_then(Value::as_object).into_iter().flatten() {
        let container_port = port.trim_end_matches("/tcp");
        for binding in bindings.as_array().into_iter().flatten() {
            let host_port = text(binding, "/HostPort");
            ports.push(match text(binding, "/HostIp").as_str() {
                "" | "0.0.0.0" => format!("{host_port}:{container_port}"),
                ip => format!("{ip}:{host_port}:{container_port}"),
            });
        }
    }
    ports.sort();
    ports.dedup();
    ports
}

/// The sandboxes' stats by name, read once
pub fn stats(names: &[String]) -> HashMap<String, Stats> {
    let mut args = vec!["stats", "--no-stream", "--format", "{{json .}}"];
    args.extend(names.iter().map(String::as_str));
    let Ok(output) = docker_output(&args) else {
        return HashMap::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_stats)
        .collect()
}

/// A line of `docker stats`: the sandbox's name and its stats
fn parse_stats(line: &str) -> Option<(String, Stats)> {
    let stats: Value = serde_json::from_str(line).ok()?;
    let name = stats["Name"].as_str()?.to_string();
    let text = |key: &str| stats[key].as_str().unwrap_or_default();
    // Used / the limit
    let memory = text("MemUsage").split(" / ").next().unwrap_or_default();
    let (received, sent) = text("NetIO").split_once(" / ").unwrap_or_default();
    let stats = Stats {
        cpu: text("CPUPerc").to_string(),
        memory: short_size(memory),
        memory_percent: text("MemPerc").to_string(),
        pids: text("PIDs").to_string(),
        received: short_size(received),
        sent: short_size(sent),
    };
    Some((name, stats))
}

/// Docker's output, with neither stdin nor stderr
fn docker_output(args: &[&str]) -> std::io::Result<Output> {
    docker::command(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
}

fn images() -> HashMap<String, ImageInfo> {
    let output = docker::output(&["image", "ls", "--format", "{{json .}}"]).unwrap_or_default();
    output
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|image| {
            let repository = image["Repository"].as_str()?;
            let tag = image["Tag"].as_str()?;
            if repository == "<none>" || tag == "<none>" {
                return None;
            }
            Some((
                format!("{repository}:{tag}"),
                ImageInfo {
                    size: short_size(image["Size"].as_str().unwrap_or_default()),
                    created: image["CreatedAt"].as_str().and_then(created_at),
                },
            ))
        })
        .collect()
}

/// Docker's `2026-11-03 10:00:00 +0000 UTC`
fn created_at(text: &str) -> Option<jiff::Timestamp> {
    let without_zone: Vec<&str> = text.split_whitespace().take(3).collect();
    jiff::fmt::strtime::parse("%Y-%m-%d %H:%M:%S %z", without_zone.join(" "))
        .ok()?
        .to_timestamp()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_containers() {
        let container: Value = serde_json::from_str(
            r#"{
                "Name": "/my-project",
                "Config": {"Labels": {"sndbx.image": "claude"}, "WorkingDir": "/workspace/web"},
                "State": {"Status": "running", "StartedAt": "2026-11-05T10:00:00.123456789Z"},
                "HostConfig": {
                    "PidsLimit": 4096,
                    "PortBindings": {"3080/tcp": [{"HostIp": "127.0.0.1", "HostPort": "3080"}],
                                     "80/tcp": [{"HostIp": "", "HostPort": "8080"}],
                                     "9000/udp": []}
                },
                "Mounts": [{"Source": "/src/web", "Destination": "/workspace/web"}],
                "NetworkSettings": {"Networks": {"sndbx-internal": {"IPAddress": "172.18.0.3"}}}
            }"#,
        )
        .unwrap();
        let sandbox = parse_sandbox(&container).unwrap();
        assert_eq!(sandbox.name, "my-project");
        assert_eq!(sandbox.image, "claude");
        assert!(sandbox.running());
        assert!(sandbox.proxied());
        assert_eq!(sandbox.proxy_ip.as_deref(), Some("172.18.0.3"));
        assert_eq!(sandbox.ip.as_deref(), Some("172.18.0.3"));

        let open: Value = serde_json::from_str(
            r#"{"Name": "/dsh", "Config": {"Labels": {"sndbx.image": "dsh"}},
                "NetworkSettings": {"Networks": {"bridge": {"IPAddress": "172.17.0.2"}}}}"#,
        )
        .unwrap();
        let open = parse_sandbox(&open).unwrap();
        assert_eq!(
            (open.proxy_ip, open.ip.as_deref()),
            (None, Some("172.17.0.2"))
        );
        assert_eq!(sandbox.pids_limit, Some(4096));
        assert_eq!(sandbox.workdir, "/workspace/web");
        assert_eq!(sandbox.ports, ["127.0.0.1:3080:3080", "8080:80"]);
        assert_eq!(
            (
                sandbox.mounts[0].source.as_str(),
                sandbox.mounts[0].destination.as_str()
            ),
            ("/src/web", "/workspace/web")
        );
        assert_eq!(
            sandbox.started,
            "2026-11-05T10:00:00.123456789Z".parse().ok()
        );

        let unlabelled: Value =
            serde_json::from_str(r#"{"Name": "/x", "Config": {"Labels": {}}}"#).unwrap();
        assert!(parse_sandbox(&unlabelled).is_none());
    }

    #[test]
    fn parses_creation_times() {
        assert_eq!(
            created_at("2026-11-03 10:00:00 +0100 CET"),
            "2026-11-03T09:00:00Z".parse().ok()
        );
        assert_eq!(created_at("yesterday"), None);
    }
}
