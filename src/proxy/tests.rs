use super::*;
use std::time::Duration;

#[test]
fn configs() {
    assert_eq!(
        config(&Hosts::Listed(vec![
            ".openai.com".to_string(),
            "pypi.org".to_string()
        ])),
        "http_port 3128\n\
         acl SSL_ports port 443\n\
         acl Safe_ports port 80 443\n\
         acl allowed dstdomain -n .openai.com pypi.org\n\
         http_access deny !Safe_ports\n\
         http_access deny CONNECT !SSL_ports\n\
         http_access allow allowed\n\
         http_access deny all\n\
         cache deny all\n\
         logformat sndbx %{%FT%T%z}tl %>a %Ss/%03>Hs %rm %ru dur=%trms in=%>st out=%<st\n\
         access_log /var/log/squid/access.log sndbx\n"
    );
}

#[test]
fn denies_all_without_hosts() {
    let config = config(&Hosts::Listed(Vec::new()));
    assert!(!config.contains("allowed"), "{config}");
    assert!(config.contains("http_access deny all\n"), "{config}");
}

#[test]
fn allows_every_host_on_the_ports() {
    assert_eq!(
        config(&Hosts::All),
        "http_port 3128\n\
         acl SSL_ports port 443\n\
         acl Safe_ports port 80 443\n\
         http_access deny !Safe_ports\n\
         http_access deny CONNECT !SSL_ports\n\
         http_access deny to_localhost\n\
         http_access deny to_linklocal\n\
         http_access allow all\n\
         http_access deny all\n\
         cache deny all\n\
         logformat sndbx %{%FT%T%z}tl %>a %Ss/%03>Hs %rm %ru dur=%trms in=%>st out=%<st\n\
         access_log /var/log/squid/access.log sndbx\n"
    );
}

#[test]
fn follows_the_last_matching_lines_then_new_ones() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;

    let log =
        std::env::temp_dir().join(format!("sndbx-follow-matching-{}.log", std::process::id()));
    std::fs::write(
        &log,
        "1 TCP_DENIED/403\n2 TCP_TUNNEL/200\n3 TCP_DENIED/403\n4 TCP_DENIED/403\n",
    )
    .unwrap();
    // In its own process group, so killing that stops tail and grep too
    let mut child = Command::new("sh")
        .args([
            "-c",
            FOLLOW_MATCHING,
            "sh",
            "2",
            log.to_str().unwrap(),
            DENIED,
        ])
        .stdout(Stdio::piped())
        .process_group(0)
        .spawn()
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    let stdout = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let next = || {
        receiver
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_default()
    };

    let earlier = [next(), next()];
    let mut file = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
    file.write_all(b"5 TCP_TUNNEL/200\n6 TCP_DENIED/403\n")
        .unwrap();
    let later = next();

    // A negative PID is the process group, which procps' kill takes only after a signal and --
    let killed = Command::new("kill")
        .args(["-TERM", "--", &format!("-{}", child.id())])
        .status()
        .unwrap();
    assert!(killed.success());
    child.wait().unwrap();
    std::fs::remove_file(&log).unwrap();
    assert_eq!(earlier, ["3 TCP_DENIED/403", "4 TCP_DENIED/403"]);
    assert_eq!(later, "6 TCP_DENIED/403");
}
