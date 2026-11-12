use super::*;

const RUN: &str = "2026-11-12T12:47:09+01:00 docker run --rm -it --name\\=my-project \
    -v /home/me/src/my\\ project:/workspace/my\\ project --env\\=COLORTERM -v /home/me/.claude:/home/ubuntu/.claude \
    --env\\=HTTP_PROXY\\=http://sndbx-proxy:3128 --security-opt\\=no-new-privileges sndbx:claude";

fn formatter(color: bool) -> Formatter {
    Formatter {
        time_zone: TimeZone::UTC,
        painter: Painter { enabled: color },
    }
}

fn format(line: &str) -> String {
    formatter(false).format(line)
}

fn parse(line: &str) -> LoggedCommand {
    LoggedCommand::parse(shell_split(line).unwrap())
}

#[test]
fn groups_repeated_options_in_rows() {
    assert_eq!(
        format(RUN),
        [
            "2026-11-12 11:47:09 docker run --rm -it sndbx:claude",
            "                           --name          my-project",
            "                           -v              /home/me/src/my project:/workspace/my project",
            "                                           /home/me/.claude:/home/ubuntu/.claude",
            "                           --env           COLORTERM",
            "                                           HTTP_PROXY=http://sndbx-proxy:3128",
            "                           --security-opt  no-new-privileges",
        ]
        .join("\n")
    );
}

#[test]
fn lays_out_any_command_in_rows() {
    assert_eq!(
        format(
            "2026-11-10T20:39:38+01:00 docker build --no-cache --build-arg TAG_PREFIX\\=sndbx -t sndbx:claude /x"
        ),
        [
            "2026-11-10 19:39:38 docker build --no-cache /x",
            "                           --build-arg  TAG_PREFIX=sndbx",
            "                           -t           sndbx:claude",
        ]
        .join("\n")
    );
    assert_eq!(
        format("2026-11-10T20:39:38+01:00 docker ps"),
        "2026-11-10 19:39:38 docker ps"
    );
}

#[test]
fn keeps_a_command_that_does_not_parse_as_logged() {
    assert_eq!(
        format("2026-11-10T20:39:38+01:00 docker run -v /a:/b 'unclosed"),
        "2026-11-10 19:39:38 docker run -v /a:/b 'unclosed"
    );
}

#[test]
fn shows_local_time() {
    let formatter = Formatter {
        time_zone: TimeZone::fixed(jiff::tz::offset(9)),
        painter: Painter { enabled: false },
    };
    assert_eq!(
        formatter.format("2026-11-12T23:30:00+01:00 docker ps"),
        "2026-11-13 07:30:00 docker ps"
    );
}

#[test]
fn keeps_lines_without_a_time() {
    for line in [
        "",
        "docker run -e A=1 -e B=2",
        "tail: sndbx.log: file truncated",
    ] {
        assert_eq!(format(line), line);
    }
}

#[test]
fn a_word_after_an_option_is_its_value() {
    let command = parse("docker run -e A=1 --init -t sndbx:claude bash -c 'ls -l'");
    let options: Vec<_> = command
        .options
        .iter()
        .map(|option| (option.key.as_str(), option.value.as_deref()))
        .collect();
    assert_eq!(command.command, ["docker", "run"]);
    assert_eq!(
        options,
        [
            ("-e", Some("A=1")),
            ("--init", None),
            ("-t", Some("sndbx:claude"))
        ]
    );
    assert_eq!(command.trailing, ["bash", "-c", "ls -l"]);
}

#[test]
fn flags_sndbx_writes_take_no_value() {
    let command = parse("docker exec -it my-project bash -c 'ls -l'");
    assert_eq!(command.options[0].value, None);
    assert_eq!(command.trailing, ["my-project", "bash", "-c", "ls -l"]);
    let command = parse("docker rm -f --volumes sndbx-proxy");
    assert!(command.options.iter().all(|option| option.value.is_none()));
    assert_eq!(command.trailing, ["sndbx-proxy"]);
    assert_eq!(
        format("2026-11-10T20:39:38+01:00 docker start -ai my-project"),
        "2026-11-10 19:39:38 docker start -ai my-project"
    );
}

#[test]
fn a_negative_number_is_a_value() {
    let command = parse("docker run --memory-swap -1 sndbx:claude");
    assert_eq!(command.options[0].value.as_deref(), Some("-1"));
    assert_eq!(command.trailing, ["sndbx:claude"]);
}

#[test]
fn a_lone_dash_is_a_value() {
    let command = parse("docker build -t x - extra");
    assert_eq!(command.options[0].value.as_deref(), Some("x"));
    assert_eq!(command.trailing, ["-", "extra"]);
}

#[test]
fn groups_values_in_the_order_keys_first_appear() {
    let command = parse("docker run --env=B -e X -v /a --env A -v /b --env=B=2 i");
    assert_eq!(
        command.values_by_key(),
        [
            ("--env", vec!["B", "A", "B=2"]),
            ("-e", vec!["X"]),
            ("-v", vec!["/a", "/b"])
        ]
    );
}

#[test]
fn colors_on_a_terminal() {
    let line = "2026-11-12T12:47:09+01:00 docker run -it --env\\=A\\=1 --env\\=B i";
    assert_eq!(
        formatter(true).format(line),
        [
            "\x1b[2m2026-11-12 11:47:09\x1b[0m \x1b[95mdocker\x1b[0m \x1b[95mrun\x1b[0m \x1b[94m-it\x1b[0m \x1b[36mi\x1b[0m",
            "                           \x1b[94m--env\x1b[0m  \x1b[33mA\x1b[0m=1",
            "                                  B",
        ]
        .join("\n")
    );
}

#[test]
fn colors_only_the_first_trailing_word() {
    let line = "2026-11-08T18:39:50+01:00 docker exec -it sndbx-proxy tail -n 10";
    assert_eq!(
        formatter(true).format(line),
        "\x1b[2m2026-11-08 17:39:50\x1b[0m \x1b[95mdocker\x1b[0m \x1b[95mexec\x1b[0m \x1b[94m-it\x1b[0m \
         \x1b[36msndbx-proxy\x1b[0m tail -n 10"
    );
}
