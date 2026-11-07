use super::*;
use std::cell::Cell;

const TUNNEL: &str = "2026-11-07T13:03:12+0000 172.18.0.3 TCP_TUNNEL/200 CONNECT api.anthropic.com:443 dur=81234ms in=1520 out=48211";

fn container(name: &str, ip: &str, started: &str) -> NetworkContainer {
    NetworkContainer {
        name: name.to_string(),
        ip: ip.to_string(),
        started: started.parse().unwrap(),
    }
}

fn formatter(
    containers: Vec<NetworkContainer>,
    color: bool,
) -> Formatter<impl FnMut() -> Vec<NetworkContainer>> {
    Formatter::new(
        move || containers.clone(),
        TimeZone::UTC,
        Painter { enabled: color },
    )
}

fn format(line: &str) -> String {
    formatter(Vec::new(), false).format(line)
}

/// Without their padding
fn columns(line: &str) -> Vec<String> {
    line.split(COLUMN_GAP)
        .map(str::trim)
        .filter(|column| !column.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[test]
fn formats_a_request() {
    let mut formatter = formatter(
        vec![container(
            "my-project",
            "172.18.0.3",
            "2026-11-07T12:00:00Z",
        )],
        false,
    );
    assert_eq!(
        formatter.format(TUNNEL),
        "2026-11-07 13:03:12  my-project       ok             81s  ↓  47KB  ↑ 1.5KB  api.anthropic.com"
    );
}

#[test]
fn shows_local_time() {
    let mut formatter = Formatter::new(
        Vec::new,
        TimeZone::fixed(jiff::tz::offset(9)),
        Painter { enabled: false },
    );
    assert!(
        formatter
            .format(TUNNEL)
            .starts_with("2026-11-07 22:03:12  ")
    );
}

#[test]
fn outcomes() {
    let outcome = |status: &str| {
        let line = TUNNEL.replace("TCP_TUNNEL/200", status);
        columns(&format(&line))[2].clone()
    };
    assert_eq!(outcome("TCP_TUNNEL/200"), "ok");
    assert_eq!(outcome("TCP_MISS/301"), "ok");
    assert_eq!(outcome("TCP_DENIED/403"), "denied");
    assert_eq!(outcome("TCP_MISS/404"), "error 404");
    assert_eq!(outcome("NONE_NONE/503"), "error 503");
    assert_eq!(outcome("TCP_MISS_ABORTED/000"), "aborted");
    assert_eq!(outcome("TCP_MISS/999"), "TCP_MISS/999");
}

#[test]
fn targets() {
    let target = |method_and_url: &str| {
        let line = TUNNEL.replace("CONNECT api.anthropic.com:443", method_and_url);
        columns(&format(&line)).pop().unwrap()
    };
    assert_eq!(target("CONNECT api.anthropic.com:443"), "api.anthropic.com");
    assert_eq!(target("CONNECT example.com:8443"), "example.com:8443");
    assert_eq!(
        target("GET http://deb.debian.org/debian/"),
        "GET http://deb.debian.org/debian/"
    );
}

#[test]
fn keeps_lines_in_another_format() {
    for line in [
        "",
        "tail: cannot open '/var/log/squid/access.log'",
        "yesterday 172.18.0.3 TCP_TUNNEL/200 CONNECT a:443 dur=1ms in=1 out=1",
        &TUNNEL.replace("dur=81234ms", "dur=-"),
        &format!("{TUNNEL} extra"),
    ] {
        assert_eq!(format(line), line);
    }
}

#[test]
fn durations() {
    assert_eq!(duration(0), "0.0s");
    assert_eq!(duration(300), "0.3s");
    assert_eq!(duration(5000), "5.0s");
    assert_eq!(duration(9960), "10s");
    assert_eq!(duration(81_234), "81s");
    assert_eq!(duration(300_000), "300s");
    assert_eq!(duration(1_210_000), "1210s");
    assert_eq!(duration(9_337_000), "9337s");
    assert_eq!(duration(999_999_000), "999999s");
}

#[test]
fn sizes() {
    assert_eq!(size(0), "0B");
    assert_eq!(size(1023), "1023B");
    assert_eq!(size(1024), "1.0KB");
    assert_eq!(size(1536), "1.5KB");
    assert_eq!(size(10_200), "10KB");
    assert_eq!(size(1_048_000), "1023KB");
    assert_eq!(size(1_048_500), "1.0MB");
    assert_eq!(size(2_200_000), "2.1MB");
    assert_eq!(size(5 << 30), "5.0GB");
}

#[test]
fn names_the_sandbox_that_had_the_address_then() {
    let mut sandboxes = Sandboxes::new(|| {
        vec![
            container("old", "172.18.0.3", "2026-11-07T10:00:00Z"),
            container("new", "172.18.0.3", "2026-11-07T12:00:00.5Z"),
            container("other", "172.18.0.4", "2026-11-07T09:00:00Z"),
        ]
    });
    let name =
        |sandboxes: &mut Sandboxes<_>, ip, time: &str| sandboxes.name(ip, time.parse().unwrap());

    assert_eq!(
        name(&mut sandboxes, "172.18.0.3", "2026-11-07T11:00:00Z").as_deref(),
        Some("old")
    );
    // In the second it started
    assert_eq!(
        name(&mut sandboxes, "172.18.0.3", "2026-11-07T12:00:00Z").as_deref(),
        Some("new")
    );
    assert_eq!(
        name(&mut sandboxes, "172.18.0.4", "2026-11-07T12:00:00Z").as_deref(),
        Some("other")
    );
    assert_eq!(
        name(&mut sandboxes, "172.18.0.3", "2026-11-07T09:00:00Z"),
        None
    );
    assert_eq!(
        name(&mut sandboxes, "172.18.0.9", "2026-11-07T12:00:00Z"),
        None
    );
}

#[test]
fn shows_the_address_without_a_sandbox() {
    assert_eq!(columns(&format(TUNNEL))[1], "172.18.0.3");
}

#[test]
fn lists_sandboxes_again_only_for_newer_requests() {
    let listings = Cell::new(0);
    let mut sandboxes = Sandboxes::new(|| {
        listings.set(listings.get() + 1);
        vec![container("a", "172.18.0.3", "2026-11-07T10:00:00Z")]
    });
    let past = "2026-11-07T11:00:00Z".parse().unwrap();
    let earlier = "2026-11-07T10:30:00Z".parse().unwrap();

    sandboxes.name("172.18.0.3", past);
    sandboxes.name("172.18.0.3", earlier);
    // Before the listing's second, so no sandbox started since has it
    sandboxes.name("172.18.0.9", past);
    assert_eq!(listings.get(), 1);
}

#[test]
fn lists_once_a_second_when_the_proxy_clock_runs_ahead() {
    let listings = Cell::new(0);
    let mut sandboxes = Sandboxes::new(|| {
        listings.set(listings.get() + 1);
        vec![container("a", "172.18.0.3", "2026-11-07T10:00:00Z")]
    });
    let ahead = "2999-01-01T00:00:00Z".parse().unwrap();
    let next_second = "2999-01-01T00:00:01Z".parse().unwrap();

    sandboxes.name("172.18.0.3", ahead);
    sandboxes.name("172.18.0.3", ahead);
    assert_eq!(listings.get(), 1);
    sandboxes.name("172.18.0.3", next_second);
    assert_eq!(listings.get(), 2);
}

#[test]
fn lists_again_once_a_second_for_unknown_addresses() {
    let listings = Cell::new(0);
    let mut sandboxes = Sandboxes::new(|| {
        listings.set(listings.get() + 1);
        if listings.get() == 1 {
            Vec::new()
        } else {
            vec![container("late", "172.18.0.3", "2999-01-01T00:00:00.5Z")]
        }
    });
    let time = "2999-01-01T00:00:00Z".parse().unwrap();

    // Started after the first listing, in the same second
    assert_eq!(sandboxes.name("172.18.0.3", time).as_deref(), Some("late"));
    assert_eq!(listings.get(), 2);
    sandboxes.name("172.18.0.9", time);
    sandboxes.name("172.18.0.9", time);
    assert_eq!(listings.get(), 2);
}

#[test]
fn keeps_sandboxes_that_stopped() {
    let listings = Cell::new(0);
    let mut sandboxes = Sandboxes::new(|| {
        listings.set(listings.get() + 1);
        if listings.get() == 1 {
            vec![container("gone", "172.18.0.3", "2026-11-07T10:00:00Z")]
        } else {
            Vec::new()
        }
    });

    assert_eq!(
        sandboxes
            .name("172.18.0.3", "2999-01-01T00:00:00Z".parse().unwrap())
            .as_deref(),
        Some("gone")
    );
    assert_eq!(
        sandboxes
            .name("172.18.0.3", "2999-01-01T00:00:01Z".parse().unwrap())
            .as_deref(),
        Some("gone")
    );
    assert_eq!(listings.get(), 2);
}

#[test]
fn pads_clients_to_an_address_or_the_widest_name_so_far() {
    let mut formatter = formatter(
        vec![
            container("short", "172.18.0.3", "2026-11-07T12:00:00Z"),
            container(
                "a-name-longer-than-ips",
                "172.18.0.4",
                "2026-11-07T12:00:00Z",
            ),
        ],
        false,
    );
    // Between the time and the outcome, with its padding
    let client = |formatter: &mut Formatter<_>, ip: &str| {
        let line = formatter.format(&TUNNEL.replace("172.18.0.3", ip));
        let after_time = line.split_once(COLUMN_GAP).unwrap().1;
        after_time
            .split_once(&format!("{COLUMN_GAP}{OK}"))
            .unwrap()
            .0
            .to_string()
    };

    assert_eq!(client(&mut formatter, "172.18.0.3"), "short          ");
    assert_eq!(client(&mut formatter, "255.255.255.255"), "255.255.255.255");
    assert_eq!(
        client(&mut formatter, "172.18.0.4"),
        "a-name-longer-than-ips"
    );
    assert_eq!(
        client(&mut formatter, "172.18.0.3"),
        "short                 "
    );
}

#[test]
fn colors() {
    let mut formatter = formatter(
        vec![container(
            "my-project",
            "172.18.0.3",
            "2026-11-07T12:00:00Z",
        )],
        true,
    );
    assert_eq!(
        formatter.format(&TUNNEL.replace("TCP_TUNNEL/200", "TCP_DENIED/403")),
        "\x1b[2m2026-11-07 13:03:12\x1b[0m  \x1b[96mmy-project     \x1b[0m  \x1b[91mdenied   \x1b[0m  \
         \x1b[2m    81s\x1b[0m  \x1b[36m↓  47KB\x1b[0m  \x1b[35m↑ 1.5KB\x1b[0m  api.anthropic.com"
    );
}

#[test]
fn colors_ports_other_than_https() {
    let mut formatter = formatter(Vec::new(), true);
    let target = |formatter: &mut Formatter<_>, url: &str| {
        let line = formatter.format(&TUNNEL.replace("api.anthropic.com:443", url));
        columns(&line).pop().unwrap()
    };

    assert_eq!(
        target(&mut formatter, "example.com:8443"),
        "example.com\x1b[91m:8443\x1b[0m"
    );
    assert_eq!(target(&mut formatter, "example.com:443"), "example.com");
}

#[test]
fn leaves_addresses_and_unknown_outcomes_uncolored() {
    let mut formatter = formatter(Vec::new(), true);
    let line = formatter.format(&TUNNEL.replace("TCP_TUNNEL/200", "TCP_MISS/999"));
    assert!(line.contains("  172.18.0.3       TCP_MISS/999  "), "{line}");
}
