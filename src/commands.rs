use std::os::unix::process::CommandExt;
use std::process::Command;

use clap::builder::NonEmptyStringValueParser;
use clap::{Args, CommandFactory, Parser, Subcommand};

use crate::builder::{BASE_NAME, Builder, IMAGES_DIR_SETTING};
use crate::docker::{self, check_container, check_docker, docker_exec, running_containers, stopped_containers};
use crate::sandbox::{MOUNT_ROOT, Options, Sandbox};
use crate::settings::{self, Settings};
use crate::{Result, error, puts};

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
        help_template = COMMAND_HELP_TEMPLATE,
        infer_long_args = true,
        args_override_self = true,
        override_usage = "sndbx run IMAGE [options] [-- DOCKER_ARGS]",
        before_help = format!(
            "Runs a sandboxed image from {}.\n\
             Mounts the current directory unless a volume is mounted in {MOUNT_ROOT}.\n\
             Passes DOCKER_ARGS to docker run after the image's docker_args.",
            settings::path().display()
        )
    )]
    Run(Run),

    /// Build an image
    #[command(
        help_template = COMMAND_HELP_TEMPLATE,
        infer_long_args = true,
        args_override_self = true,
        override_usage = "sndbx build IMAGE | --all",
        before_help = format!(
            "Builds an image from {IMAGES_DIR_SETTING}/IMAGE, always without the docker cache.\n\
             Builds {BASE_NAME} first if the image needs it and it doesn't exist yet."
        )
    )]
    Build(Build),

    /// Run a command in a running sandbox
    #[command(
        help_template = COMMAND_HELP_TEMPLATE,
        infer_long_args = true,
        args_override_self = true,
        override_usage = "sndbx exec IMAGE CONTAINER COMMAND [ARGS] | IMAGE --list",
        before_help = "Runs COMMAND in a running sandbox of IMAGE, like docker exec -i,\n\
                       with -t when stdin is a terminal.\n\
                       Options go before COMMAND, everything from COMMAND on goes to it."
    )]
    Exec(Exec),

    /// Restart a stopped sandbox
    #[command(
        help_template = COMMAND_HELP_TEMPLATE,
        infer_long_args = true,
        args_override_self = true,
        override_usage = "sndbx resume IMAGE CONTAINER | IMAGE --list",
        before_help = "Restarts a stopped sandbox of IMAGE, kept by run --keep, and attaches to it,\n\
                       like docker start -ai."
    )]
    Resume(Resume),

    /// Follow the log of docker commands
    #[command(
        help_template = COMMAND_HELP_TEMPLATE,
        infer_long_args = true,
        args_override_self = true,
        override_usage = "sndbx logs [options]",
        before_help = format!(
            "Follows {}, the docker commands sndbx ran, like tail -f.",
            docker::log_path().display()
        )
    )]
    Logs(Logs),
}

#[derive(Args)]
struct Run {
    /// The image's name in the settings
    #[arg(required_unless_present = "list")]
    image: Option<String>,

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

    /// Print image names
    #[arg(short, long)]
    list: bool,

    /// More docker run arguments
    #[arg(last = true, value_name = "DOCKER_ARGS")]
    docker_args: Vec<String>,
}

#[derive(Args)]
struct Build {
    /// The image's directory in `images_dir`
    #[arg(required_unless_present_any = ["all", "list"], conflicts_with = "all")]
    image: Option<String>,

    #[arg(short, long, help = format!("Build {BASE_NAME}, then every other image"))]
    all: bool,

    /// Print image names
    #[arg(short, long)]
    list: bool,
}

#[derive(Args)]
struct Exec {
    /// The image's name in the settings
    image: String,

    /// A running container's name
    #[arg(required_unless_present = "list", conflicts_with = "list")]
    container: Option<String>,

    /// The command and its arguments
    #[arg(
        required_unless_present = "list",
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

    /// Print IMAGE's running containers
    #[arg(short, long)]
    list: bool,
}

#[derive(Args)]
struct Resume {
    /// The image's name in the settings
    image: String,

    /// A stopped container's name
    #[arg(required_unless_present = "list", conflicts_with = "list")]
    container: Option<String>,

    /// Print IMAGE's stopped containers
    #[arg(short, long)]
    list: bool,
}

#[derive(Args)]
struct Logs {
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

pub fn cli() -> Result<()> {
    let Some(command) = Cli::parse().command else {
        let mut out = std::io::stdout().lock();
        let _ = Cli::command().write_help(&mut out);
        return Ok(());
    };

    match command {
        Commands::Run(args) => run(args),
        Commands::Build(args) => build(&args),
        Commands::Exec(args) => exec(args),
        Commands::Resume(args) => resume(args),
        Commands::Logs(args) => logs(&args),
    }
}

fn run(args: Run) -> Result<()> {
    if args.list {
        if settings::path().exists() {
            print_lines(&Settings::default().image_names()?);
        }
        return Ok(());
    }

    let image = Settings::default().image(&args.image.unwrap_or_default())?;
    let options = Options {
        volumes: args.volumes,
        ports: args.ports,
        workdir: args.workdir,
        keep: args.keep,
        docker_args: args.docker_args,
    };
    Sandbox::new(image, options)?.call()
}

fn build(args: &Build) -> Result<()> {
    let builder = Builder::new(&mut Settings::default())?;
    if args.list {
        print_lines(&builder.image_names()?);
        return Ok(());
    }

    builder.call(args.image.as_deref())
}

fn exec(args: Exec) -> Result<()> {
    if args.list {
        check_docker()?;
        print_lines(&running_containers(&args.image)?);
        return Ok(());
    }

    let image = &args.image;
    let container = args.container.unwrap_or_default();
    Settings::default().image(image)?;
    check_docker()?;
    check_container(
        &container,
        &running_containers(image)?,
        &format!("running {image} sandbox"),
    )?;

    // Without a terminal on stdin docker refuses -t, which breaks pipes and scripts
    let tty = if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        "-it"
    } else {
        "-i"
    };
    let mut docker_args = vec![String::from("exec"), String::from(tty)];
    docker_args.extend(args.user.map(|user| format!("--user={user}")));
    docker_args.push(container);
    docker_args.extend(args.command);
    docker_exec(&docker_args)
}

fn resume(args: Resume) -> Result<()> {
    if args.list {
        check_docker()?;
        print_lines(&stopped_containers(&args.image)?);
        return Ok(());
    }

    let image = &args.image;
    let container = args.container.unwrap_or_default();
    Settings::default().image(image)?;
    check_docker()?;
    check_container(
        &container,
        &stopped_containers(image)?,
        &format!("stopped {image} sandbox"),
    )?;
    docker_exec(&[String::from("start"), String::from("-ai"), container])
}

fn logs(args: &Logs) -> Result<()> {
    const TAIL: &str = "tail";

    let log_path = docker::log_path();
    if !log_path.exists() {
        return error(format!("no log yet at {}", log_path.display()));
    }

    let e = Command::new(TAIL)
        .args(["-n", &args.lines.to_string(), "-f"])
        .arg(&log_path)
        .exec();
    if e.kind() == std::io::ErrorKind::NotFound {
        return error(format!("{TAIL} is not installed"));
    }
    error(format!("can't run {TAIL}: {e}"))
}

fn print_lines(lines: &[String]) {
    for line in lines {
        puts(line);
    }
}

/// A line count of 0 or more, written like Ruby's `Integer`: decimal, `0b` binary, `0` octal
/// or `0x` hexadecimal, with an optional sign and single underscores between digits.
fn line_count(value: &str) -> std::result::Result<u64, String> {
    match parse_integer(value) {
        None => Err(String::from("expected a number")),
        Some(count) => u64::try_from(count).map_err(|_| String::from("expected a line count of 0 or more")),
    }
}

fn parse_integer(value: &str) -> Option<i128> {
    let (negative, unsigned) = match value.as_bytes().first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    let lower = unsigned.to_ascii_lowercase();

    let (radix, digits) = if let Some(digits) = lower.strip_prefix("0b") {
        (2, digits)
    } else if let Some(digits) = lower.strip_prefix("0x") {
        (16, digits)
    } else if lower == "0" {
        (10, "0")
    } else if let Some(digits) = lower.strip_prefix('0') {
        (8, digits)
    } else {
        (10, lower.as_str())
    };

    // Digits, with single underscores only between them
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
        return None;
    }

    let number = digits.chars().filter(|&c| c != '_').try_fold(0_i128, |number, c| {
        number
            .checked_mul(i128::from(radix))?
            .checked_add(i128::from(c.to_digit(radix)?))
    })?;

    Some(if negative { -number } else { number })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn integers() {
        assert_eq!(parse_integer("10"), Some(10));
        assert_eq!(parse_integer("-0"), Some(0));
        assert_eq!(parse_integer("+3"), Some(3));
        assert_eq!(parse_integer("1_000"), Some(1000));
        assert_eq!(parse_integer("010"), Some(8));
        assert_eq!(parse_integer("0b11"), Some(3));
        assert_eq!(parse_integer("0X1f"), Some(31));
        assert_eq!(parse_integer("99999999999999999999"), Some(99_999_999_999_999_999_999));
        for invalid in ["", "x", "1__0", "_1", "1_", "0o10", "1e3", " 5", "08", "-"] {
            assert_eq!(parse_integer(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn line_counts() {
        assert_eq!(line_count("0x10"), Ok(16));
        assert_eq!(line_count("-0"), Ok(0));
        assert!(line_count("-1").is_err());
        assert!(line_count("x").is_err());
    }
}
