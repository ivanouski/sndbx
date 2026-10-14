use std::os::unix::process::CommandExt;
use std::process::Command;

use crate::builder::{BASE_NAME, Builder, IMAGES_DIR_SETTING};
use crate::docker::{self, check_container, check_docker, docker_exec, running_containers, stopped_containers};
use crate::options::{Event, Parser, Switch, integer};
use crate::sandbox::{MOUNT_ROOT, Options, Sandbox};
use crate::settings::{self, Settings};
use crate::{Error, Result, VERSION, error, exit_with, puts};

const COMMANDS: [&str; 5] = ["run", "build", "exec", "resume", "logs"];

pub fn cli(mut args: Vec<String>) -> Result<()> {
    if args.is_empty() {
        args.push(String::from("-h"));
    }

    let separators = lines(&[
        "",
        "Commands:",
        "    run IMAGE                       Run a sandboxed image",
        "    build IMAGE                     Build an image",
        "    exec IMAGE CONTAINER COMMAND    Run a command in a running sandbox",
        "    resume IMAGE CONTAINER          Restart a stopped sandbox",
        "    logs                            Follow the log of docker commands",
        "",
        "Run 'sndbx command --help' for a command's options.",
        "",
    ]);
    let mut parser = Parser::new(
        args,
        "Usage: sndbx command [options]",
        &separators,
        vec![Switch::flag('V', "version", &["Print version"]), Switch::help()],
    );

    // Stop at the command, its options are its own
    match parser.next_event()? {
        Some(Event::Opt { long: "version", .. }) => exit_with(&format!("sndbx {VERSION}\n")),
        Some(Event::Opt { .. }) => exit_with(&parser.help()),
        Some(Event::Arg(arg)) => parser.unshift(arg),
        Some(Event::End) | None => {}
    }

    let mut args = parser.rest();
    if args.is_empty() {
        return Err(Error::Abort(parser.help()));
    }
    let name = args.remove(0);

    match name.as_str() {
        "run" => run(args),
        "build" => build(args),
        "exec" => exec(args),
        "resume" => resume(args),
        "logs" => logs(args),
        _ => error(format!(
            "unknown command '{name}', expected one of: {}",
            COMMANDS.join(", ")
        )),
    }
}

fn run(mut args: Vec<String>) -> Result<()> {
    let docker_args = match args.iter().position(|arg| arg == "--") {
        Some(separator) => args.split_off(separator).split_off(1),
        None => Vec::new(),
    };

    let path = settings::path();
    let separators = lines(&[
        "",
        &format!("Runs a sandboxed image from {}.", path.display()),
        &format!("Mounts the current directory unless a volume is mounted in {MOUNT_ROOT}."),
        "Passes DOCKER_ARGS to docker run after the image's docker_args.",
        "",
    ]);
    let mut parser = Parser::new(
        args,
        "Usage: sndbx run IMAGE [options] [-- DOCKER_ARGS]",
        &separators,
        vec![
            Switch::with_arg(
                'v',
                "volume",
                "SRC[:DST[:OPTS]]",
                &[
                    &format!("Mount SRC at DST, {MOUNT_ROOT}/<basename> by default,"),
                    "with docker's OPTS, e.g. ro, repeatable",
                ],
            ),
            Switch::with_arg(
                'w',
                "workdir",
                "NAME",
                &[
                    &format!("Start in {MOUNT_ROOT}/<NAME>, or NAME if it's absolute,"),
                    &format!("the last directory volume in {MOUNT_ROOT} by default"),
                ],
            ),
            Switch::with_arg(
                'p',
                "publish",
                "PORT",
                &["Publish a port as HOST:CONTAINER, PORT means PORT:PORT, repeatable"],
            ),
            Switch::flag(
                'k',
                "keep",
                &["Keep the container after it exits, docker run's --rm otherwise removes it"],
            ),
            Switch::flag('l', "list", &["Print image names"]),
            Switch::help(),
        ],
    );

    let mut options = Options {
        volumes: Vec::new(),
        ports: Vec::new(),
        workdir: None,
        keep: false,
        docker_args,
    };
    let mut positionals = Vec::new();
    while let Some(event) = parser.next_event()? {
        match event {
            Event::Opt {
                long: "volume", value, ..
            } => options.volumes.extend(value),
            Event::Opt {
                long: "workdir", value, ..
            } => options.workdir = value,
            Event::Opt {
                long: "publish", value, ..
            } => options.ports.extend(value),
            Event::Opt { long: "keep", .. } => options.keep = true,
            Event::Opt { long: "list", .. } => {
                if settings::path().exists() {
                    print_lines(&Settings::default().image_names()?);
                }
                exit_with("");
            }
            Event::Opt { .. } => exit_with(&parser.help()),
            Event::Arg(arg) => positionals.push(arg),
            Event::End => positionals.extend(parser.rest()),
        }
    }

    let [image] = positionals.as_slice() else {
        return error("expected exactly one image");
    };

    let image = Settings::default().image(image)?;
    Sandbox::new(image, options)?.call()
}

fn build(args: Vec<String>) -> Result<()> {
    let separators = lines(&[
        "",
        &format!("Builds an image from {IMAGES_DIR_SETTING}/IMAGE, always without the docker cache."),
        &format!("Builds {BASE_NAME} first if the image needs it and it doesn't exist yet."),
        "",
    ]);
    let mut parser = Parser::new(
        args,
        "Usage: sndbx build IMAGE | --all",
        &separators,
        vec![
            Switch::flag('a', "all", &[&format!("Build {BASE_NAME}, then every other image")]),
            Switch::flag('l', "list", &["Print image names"]),
            Switch::help(),
        ],
    );

    let mut all = false;
    let mut positionals = Vec::new();
    while let Some(event) = parser.next_event()? {
        match event {
            Event::Opt { long: "all", .. } => all = true,
            Event::Opt { long: "list", .. } => {
                print_lines(&Builder::new(&mut Settings::default())?.image_names()?);
                exit_with("");
            }
            Event::Opt { .. } => exit_with(&parser.help()),
            Event::Arg(arg) => positionals.push(arg),
            Event::End => positionals.extend(parser.rest()),
        }
    }

    let valid = if all {
        positionals.is_empty()
    } else {
        positionals.len() == 1
    };
    if !valid {
        return error("expected exactly one image, or --all");
    }

    Builder::new(&mut Settings::default())?.call(positionals.first().map(String::as_str))
}

fn exec(args: Vec<String>) -> Result<()> {
    let separators = lines(&[
        "",
        "Runs COMMAND in a running sandbox of IMAGE, like docker exec -i,",
        "with -t when stdin is a terminal.",
        "Options go before COMMAND, everything from COMMAND on goes to it.",
        "",
    ]);
    let mut parser = Parser::new(
        args,
        "Usage: sndbx exec IMAGE CONTAINER COMMAND [ARGS] | IMAGE --list",
        &separators,
        vec![
            Switch::with_arg(
                'u',
                "user",
                "USER",
                &["Run COMMAND as USER, the container's user by default"],
            ),
            Switch::flag('l', "list", &["Print IMAGE's running containers"]),
            Switch::help(),
        ],
    );

    let mut user = None;
    let mut list = false;
    let mut positionals: Vec<String> = Vec::new();

    // Stop at COMMAND, the third positional, so its own options reach it
    while let Some(event) = parser.next_event()? {
        match event {
            Event::Opt {
                long: "user", value, ..
            } => {
                let value = value.unwrap_or_default();
                if value.is_empty() {
                    return error("expected a user");
                }
                user = Some(value);
            }
            Event::Opt { long: "list", .. } => list = true,
            Event::Opt { .. } => exit_with(&parser.help()),
            Event::Arg(arg) => {
                if positionals.len() == 2 {
                    parser.unshift(arg);
                    break;
                }
                positionals.push(arg);
            }
            Event::End => break,
        }
    }

    // -- ends the options, the words after it start with the missing positionals
    let mut command = parser.rest();
    while positionals.len() < 2 && !command.is_empty() {
        positionals.push(command.remove(0));
    }

    if list {
        let [image] = positionals.as_slice() else {
            return error("expected exactly one image");
        };
        if !command.is_empty() {
            return error("expected exactly one image");
        }

        check_docker()?;
        print_lines(&running_containers(image)?);
        return Ok(());
    }

    let [image, container] = positionals.as_slice() else {
        return error("expected an image, a container and a command");
    };
    if command.is_empty() {
        return error("expected an image, a container and a command");
    }

    Settings::default().image(image)?;
    check_docker()?;
    check_container(
        container,
        &running_containers(image)?,
        &format!("running {image} sandbox"),
    )?;

    // Without a terminal on stdin docker refuses -t, which breaks pipes and scripts
    let tty = if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        "-it"
    } else {
        "-i"
    };
    let mut args = vec![String::from("exec"), String::from(tty)];
    args.extend(user.map(|user| format!("--user={user}")));
    args.push(container.clone());
    args.extend(command);
    docker_exec(&args)
}

fn resume(args: Vec<String>) -> Result<()> {
    let separators = lines(&[
        "",
        "Restarts a stopped sandbox of IMAGE, kept by run --keep, and attaches to it,",
        "like docker start -ai.",
        "",
    ]);
    let mut parser = Parser::new(
        args,
        "Usage: sndbx resume IMAGE CONTAINER | IMAGE --list",
        &separators,
        vec![
            Switch::flag('l', "list", &["Print IMAGE's stopped containers"]),
            Switch::help(),
        ],
    );

    let mut list = false;
    let mut positionals = Vec::new();
    while let Some(event) = parser.next_event()? {
        match event {
            Event::Opt { long: "list", .. } => list = true,
            Event::Opt { .. } => exit_with(&parser.help()),
            Event::Arg(arg) => positionals.push(arg),
            Event::End => positionals.extend(parser.rest()),
        }
    }

    if list {
        let [image] = positionals.as_slice() else {
            return error("expected exactly one image");
        };

        check_docker()?;
        print_lines(&stopped_containers(image)?);
        return Ok(());
    }

    let [image, container] = positionals.as_slice() else {
        return error("expected an image and a container");
    };

    Settings::default().image(image)?;
    check_docker()?;
    check_container(
        container,
        &stopped_containers(image)?,
        &format!("stopped {image} sandbox"),
    )?;
    docker_exec(&[String::from("start"), String::from("-ai"), container.clone()])
}

fn logs(args: Vec<String>) -> Result<()> {
    const TAIL: &str = "tail";
    let mut lines_count: i128 = 10;

    let log_path = docker::log_path();
    let separators = lines(&[
        "",
        &format!(
            "Follows {}, the docker commands sndbx ran, like tail -f.",
            log_path.display()
        ),
        "",
    ]);
    let mut parser = Parser::new(
        args,
        "Usage: sndbx logs [options]",
        &separators,
        vec![
            Switch::with_arg(
                'n',
                "lines",
                "N",
                &[&format!("Start with the last N lines, {lines_count} by default")],
            ),
            Switch::help(),
        ],
    );

    let mut positionals = Vec::new();
    while let Some(event) = parser.next_event()? {
        match event {
            Event::Opt {
                long: "lines",
                value,
                given,
            } => {
                let count = integer(&value.unwrap_or_default(), &given)?;
                if count < 0 {
                    return error("expected a line count of 0 or more");
                }
                lines_count = count;
            }
            Event::Opt { .. } => exit_with(&parser.help()),
            Event::Arg(arg) => positionals.push(arg),
            Event::End => positionals.extend(parser.rest()),
        }
    }

    if !positionals.is_empty() {
        return error("expected no arguments");
    }
    if !log_path.exists() {
        return error(format!("no log yet at {}", log_path.display()));
    }

    let e = Command::new(TAIL)
        .args(["-n", &lines_count.to_string(), "-f"])
        .arg(&log_path)
        .exec();
    if e.kind() == std::io::ErrorKind::NotFound {
        return error(format!("{TAIL} is not installed"));
    }
    error(format!("can't run {TAIL}: {e}"))
}

fn lines(lines: &[&str]) -> Vec<String> {
    lines.iter().map(ToString::to_string).collect()
}

fn print_lines(lines: &[String]) {
    for line in lines {
        puts(line);
    }
}
