use std::io::IsTerminal;
use std::os::unix::process::CommandExt;
use std::process::Command;

use anyhow::{Context, bail};
use clap::builder::NonEmptyStringValueParser;
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};

use crate::builder::{BASE_NAME, Builder};
use crate::docker::{
    self, check_container, check_docker, image_containers, image_exists, running_containers, stopped_containers,
};
use crate::proxy::{self, Hosts, Proxy};
use crate::sandbox::{MOUNT_ROOT, Options, Sandbox};
use crate::settings::{self, Allow, IMAGES_DIR_SETTING, Settings};
use crate::{Result, allowlist, paths, print_line, print_status, unique};

/// The usage first, like sndbx's help always had it
const HELP_TEMPLATE: &str = "{usage-heading} {usage}\n\n{all-args}{after-help}";
/// The usage, then what the command does
const COMMAND_HELP_TEMPLATE: &str = "{usage-heading} {usage}\n\n{before-help}{all-args}";
const DEFAULT_LINES: u64 = 10;

#[derive(Parser)]
#[command(
    name = "sndbx",
    version,
    override_usage = "sndbx command [options]",
    after_help = "Run 'sndbx command --help' for a command's options.",
    help_template = HELP_TEMPLATE,
    disable_help_subcommand = true,
    infer_long_args = true,
    args_override_self = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a sandboxed image
    #[command(
        override_usage = "sndbx run IMAGE [options] [-- DOCKER_ARGS]",
        before_help = format!(
            "Runs a sandboxed image from {}.\n\
             Mounts the current directory unless a volume is mounted in {MOUNT_ROOT}.\n\
             Passes DOCKER_ARGS to docker run after the docker_args in the settings.\n\
             Builds the image first if it doesn't exist and `{IMAGES_DIR_SETTING}` has a Dockerfile for its tag; \
             pull or build any other image yourself.",
            settings::path().display()
        )
    )]
    Run(Run),

    /// Build images
    #[command(
        override_usage = "sndbx build [--rebuild-base] IMAGE...",
        before_help = format!(
            "Builds images from `{IMAGES_DIR_SETTING}`/IMAGE in turn, always without the docker cache.\n\
             Builds {BASE_NAME} first with --rebuild-base, or if an image needs it and it doesn't exist yet."
        )
    )]
    Build(Build),

    /// Remove images
    #[command(
        override_usage = "sndbx rm IMAGE...",
        before_help = format!(
            "Removes images, like docker image rm, and comments out their entries in {}.\n\
             Keeps their config directories. Refuses images that containers use, remove those with docker rm first.",
            settings::path().display()
        )
    )]
    Rm(Rm),

    /// Run a command in a running sandbox
    #[command(
        override_usage = "sndbx exec IMAGE CONTAINER [options] COMMAND [ARGS]",
        before_help = "Runs COMMAND in a running sandbox of IMAGE, like docker exec -i,\n\
                       with -t when stdin is a terminal.\n\
                       Options go before COMMAND, everything from COMMAND on goes to it."
    )]
    Exec(Exec),

    /// Restart a stopped sandbox
    #[command(
        override_usage = "sndbx resume IMAGE CONTAINER",
        before_help = "Restarts a stopped sandbox of IMAGE, kept by run --keep, and attaches to it,\n\
                       like docker start -ai."
    )]
    Resume(Resume),

    /// Print image or sandbox names
    #[command(
        disable_help_subcommand = true,
        override_usage = "sndbx list WHAT",
        before_help = "Prints image or sandbox names, one per line."
    )]
    List(List),

    /// Follow the docker commands sndbx ran or the proxy's requests
    #[command(
        disable_help_subcommand = true,
        override_usage = "sndbx logs WHAT [options]",
        before_help = "Follows a log, like tail -f."
    )]
    Logs(Logs),
}

#[derive(Args)]
struct Run {
    /// The image's name in the settings
    image: String,

    #[arg(
        short,
        long = "volume",
        value_name = "SRC[:DST[:OPTS]]",
        allow_hyphen_values = true,
        help = format!("Mount SRC at DST, {MOUNT_ROOT}/<basename> by default, with docker's OPTS, e.g. ro, repeatable")
    )]
    volumes: Vec<String>,

    #[arg(
        short,
        long,
        value_name = "NAME",
        allow_hyphen_values = true,
        help = format!(
            "Start in {MOUNT_ROOT}/<NAME>, or NAME if it's absolute, the last directory volume in {MOUNT_ROOT} by default"
        )
    )]
    workdir: Option<String>,

    /// Publish a port as HOST:CONTAINER, PORT means PORT:PORT, repeatable
    #[arg(short, long = "publish", value_name = "PORT", allow_hyphen_values = true)]
    ports: Vec<String>,

    /// Keep the container after it exits, docker run's --rm otherwise removes it
    #[arg(short, long)]
    keep: bool,

    /// More docker run arguments
    #[arg(last = true, value_name = "DOCKER_ARGS")]
    docker_args: Vec<String>,
}

#[derive(Args)]
struct Build {
    /// The images' directories in `images_dir`
    #[arg(required_unless_present = "rebuild_base", value_name = "IMAGE")]
    images: Vec<String>,

    #[arg(short = 'b', long, help = format!("Rebuild {BASE_NAME} before the images, even if it exists"))]
    rebuild_base: bool,
}

#[derive(Args)]
struct Rm {
    /// The images' names in the settings, or of built images that aren't in them
    #[arg(required = true, value_name = "IMAGE")]
    images: Vec<String>,
}

#[derive(Args)]
struct Exec {
    /// The image's name in the settings
    image: String,

    /// A running container's name
    container: String,

    /// The command and its arguments
    #[arg(
        required = true,
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "COMMAND"
    )]
    command: Vec<String>,

    /// Run COMMAND as USER, the container's user by default
    #[arg(
        short,
        long,
        value_name = "USER",
        allow_hyphen_values = true,
        value_parser = NonEmptyStringValueParser::new()
    )]
    user: Option<String>,
}

#[derive(Args)]
struct Resume {
    /// The image's name in the settings
    image: String,

    /// A stopped container's name
    container: String,
}

#[derive(Args)]
struct List {
    #[command(subcommand)]
    what: Option<ListWhat>,
}

#[derive(Subcommand)]
enum ListWhat {
    /// Images in the settings, which run, exec and resume take
    #[command(override_usage = "sndbx list configured")]
    Configured,

    /// Images in `images_dir`, which build takes
    #[command(override_usage = "sndbx list buildable")]
    Buildable,

    /// IMAGE's running sandboxes, which exec takes
    #[command(override_usage = "sndbx list running IMAGE")]
    Running(ListImage),

    /// IMAGE's stopped sandboxes, which resume takes
    #[command(override_usage = "sndbx list stopped IMAGE")]
    Stopped(ListImage),
}

#[derive(Args)]
struct ListImage {
    /// The image's name in the settings
    image: String,
}

#[derive(Args)]
struct Logs {
    #[command(subcommand)]
    what: Option<LogsWhat>,
}

#[derive(Subcommand)]
enum LogsWhat {
    /// The docker commands sndbx ran
    #[command(
        override_usage = "sndbx logs commands [options]",
        before_help = format!("Follows {}, the docker commands sndbx ran.", docker::log_path().display())
    )]
    Commands(Lines),

    /// The requests the proxy let through or denied
    #[command(
        override_usage = "sndbx logs proxy [options]",
        before_help = "Follows the access log of the running proxy, which goes when the proxy is replaced."
    )]
    Proxy(ProxyLogs),
}

#[derive(Args)]
struct ProxyLogs {
    #[command(flatten)]
    lines: Lines,

    /// Only the requests the proxy denied
    #[arg(short, long)]
    denied: bool,
}

#[derive(Args)]
struct Lines {
    /// Start with the last N lines
    #[arg(
        short = 'n',
        long,
        value_name = "N",
        default_value_t = DEFAULT_LINES,
        allow_hyphen_values = true,
        value_parser = line_count
    )]
    lines: u64,
}

/// The CLI with the settings every command shares
fn command() -> clap::Command {
    Cli::command().mut_subcommands(with_shared_settings)
}

fn with_shared_settings(command: clap::Command) -> clap::Command {
    command
        .help_template(COMMAND_HELP_TEMPLATE)
        .infer_long_args(true)
        .args_override_self(true)
        .mut_subcommands(with_shared_settings)
}

pub fn cli() -> Result<()> {
    let matches = command().get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    let Some(subcommand) = cli.command else {
        let mut out = std::io::stdout().lock();
        let _ = command().write_help(&mut out);
        return Ok(());
    };
    paths::check_home()?;

    match subcommand {
        Commands::Run(args) => run(args),
        Commands::Build(args) => build(&args),
        Commands::Rm(args) => rm(&args),
        Commands::Exec(args) => exec(&args),
        Commands::Resume(args) => resume(&args),
        Commands::List(args) => list(args),
        Commands::Logs(args) => logs(args),
    }
}

fn run(args: Run) -> Result<()> {
    let settings = Settings::load()?;
    let image = settings.image(&args.image)?;
    let proxy = match settings.proxy() {
        Some(proxy) if image.proxy => Some(Proxy {
            image: proxy.image.clone(),
            hosts: match proxy.allow {
                Allow::Listed => Hosts::Listed(allowlist::load()?),
                Allow::All => Hosts::All,
            },
        }),
        _ => None,
    };
    let options = Options {
        volumes: args.volumes,
        ports: args.ports,
        workdir: args.workdir,
        keep: args.keep,
        docker_args: args.docker_args,
        proxy,
    };
    let sandbox = Sandbox::new(image, options)?;
    check_docker()?;
    Builder::new(&settings).build_missing(sandbox.image())?;
    sandbox.run()
}

fn build(args: &Build) -> Result<()> {
    Builder::new(&Settings::load_or_default()?).build(&args.images, args.rebuild_base)
}

/// An image `rm` checked
struct Removal<'a> {
    name: &'a str,
    tag: String,
    exists: bool,
}

/// Checks every image before removing any, so a typo or a container stops `rm` before it changes anything
fn rm(args: &Rm) -> Result<()> {
    let settings = Settings::load_or_default()?;
    if args.images.iter().any(|name| name == BASE_NAME) {
        bail!("{BASE_NAME} can't be removed, the other images are built on it");
    }
    let names = unique(args.images.iter().map(String::as_str));

    check_docker()?;
    let removals = names
        .iter()
        .map(|name| check_removal(&settings, name, &names))
        .collect::<Result<Vec<_>>>()?;
    remove_images(&removals)
}

/// The image as `rm` removes it along with `names`, an error when it's unknown, another image not in `names`
/// runs its tag too, or containers use it
fn check_removal<'a>(settings: &Settings, name: &'a str, names: &[&str]) -> Result<Removal<'a>> {
    let configured_tag = settings.configured_tag(name);
    let tag = configured_tag.clone().unwrap_or_else(|| settings.default_tag(name));
    let exists = image_exists(&tag);
    if configured_tag.is_none() && !exists {
        bail!(
            "unknown image '{name}', it's neither in {} nor built as {tag}",
            settings::path().display()
        );
    }

    let others: Vec<&str> = settings
        .names_running(&tag)
        .into_iter()
        .filter(|other| !names.contains(other))
        .collect();
    if !others.is_empty() {
        bail!("{tag} is also run by {}, rm them together", others.join(", "));
    }
    if exists {
        let containers = image_containers(&tag)?;
        if !containers.is_empty() {
            bail!(
                "{tag} is used by containers: {}, remove them with docker rm first",
                containers.join(", ")
            );
        }
    }
    Ok(Removal { name, tag, exists })
}

/// Removes the checked images and comments out their entries, images that share a tag remove it once
fn remove_images(removals: &[Removal]) -> Result<()> {
    let mut removed: Vec<&str> = Vec::new();
    for removal in removals {
        let tag = removal.tag.as_str();
        if !removed.contains(&tag) {
            if removal.exists {
                if !docker::run(&["image", "rm", tag]) {
                    bail!("failed to remove {tag}");
                }
                print_status(&format!("Removed {tag}"));
            } else {
                print_status(&format!("{tag} doesn't exist"));
            }
            removed.push(tag);
        }
        comment_out(removal.name);
    }
    Ok(())
}

/// Comments out the image's entry, only warning when it can't since the image is removed
fn comment_out(name: &str) {
    let path = settings::path();
    match settings::comment_out_image(name) {
        Ok(false) => {}
        Ok(true) => print_status(&format!("Commented out {name} in {}", path.display())),
        Err(e) => eprintln!(
            "Warning: can't comment out {name} in the settings: {e:#}.\nRemove its entry from {} yourself.",
            path.display()
        ),
    }
}

fn exec(args: &Exec) -> Result<()> {
    let container = &args.container;
    check_sandbox(&args.image, container, "running", running_containers)?;

    // Without a terminal on stdin docker refuses -t, which breaks pipes and scripts
    let tty = if std::io::stdin().is_terminal() { "-it" } else { "-i" };
    let user = args.user.as_ref().map(|user| format!("--user={user}"));

    let mut docker_args = vec!["exec", tty];
    docker_args.extend(user.as_deref());
    docker_args.push(container);
    docker_args.extend(args.command.iter().map(String::as_str));
    docker::exec(&docker_args)
}

fn resume(args: &Resume) -> Result<()> {
    let container = &args.container;
    check_sandbox(&args.image, container, "stopped", stopped_containers)?;
    docker::exec(&["start", "-ai", container])
}

/// Checks that the image is in the settings and the container is one of its sandboxes in `state`
fn check_sandbox(image: &str, container: &str, state: &str, containers: fn(&str) -> Result<Vec<String>>) -> Result<()> {
    Settings::load()?.image(image)?;
    check_docker()?;
    check_container(container, &containers(image)?, &format!("{state} {image} sandbox"))
}

fn list(args: List) -> Result<()> {
    let names = match args.what {
        None => {
            let mut out = std::io::stdout().lock();
            if let Some(list) = command().find_subcommand_mut("list") {
                let _ = list.write_help(&mut out);
            }
            return Ok(());
        }
        Some(ListWhat::Configured) => Settings::load_or_default()?.image_names(),
        Some(ListWhat::Buildable) => Builder::new(&Settings::load_or_default()?).image_names()?,
        Some(ListWhat::Running(args)) => {
            check_docker()?;
            running_containers(&args.image)?
        }
        Some(ListWhat::Stopped(args)) => {
            check_docker()?;
            stopped_containers(&args.image)?
        }
    };
    for name in &names {
        print_line(name);
    }
    Ok(())
}

fn logs(args: Logs) -> Result<()> {
    match args.what {
        None => {
            let mut out = std::io::stdout().lock();
            if let Some(logs) = command().find_subcommand_mut("logs") {
                let _ = logs.write_help(&mut out);
            }
            Ok(())
        }
        Some(LogsWhat::Commands(args)) => command_log(args.lines),
        Some(LogsWhat::Proxy(args)) => {
            check_docker()?;
            proxy::follow_log(args.lines.lines, args.denied)
        }
    }
}

fn command_log(lines: u64) -> Result<()> {
    const TAIL: &str = "tail";

    let log_path = docker::log_path();
    if !log_path.exists() {
        bail!("no log yet at {}", log_path.display());
    }

    let e = Command::new(TAIL)
        .args(["-n", &lines.to_string(), "-f"])
        .arg(&log_path)
        .exec();
    if e.kind() == std::io::ErrorKind::NotFound {
        bail!("{TAIL} is not installed");
    }
    Err(e).context(format!("can't run {TAIL}"))
}

fn line_count(value: &str) -> Result<u64, String> {
    let count: i64 = value.parse().map_err(|_| "expected a number")?;
    u64::try_from(count).map_err(|_| "expected a line count of 0 or more".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_is_valid() {
        command().debug_assert();
    }

    #[test]
    fn line_counts() {
        assert_eq!(line_count("10"), Ok(10));
        assert_eq!(line_count("+3"), Ok(3));
        assert_eq!(line_count("-0"), Ok(0));
        assert!(line_count("-1").is_err());
        assert!(line_count("x").is_err());
        assert!(line_count("0x10").is_err());
    }
}
