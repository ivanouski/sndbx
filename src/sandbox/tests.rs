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
            host_paths: false,
        },
        keep,
        docker_args: vec!["--env=FOO=bar".to_string()],
        volumes: vec![Volume {
            source: PathBuf::from("/src/app"),
            target: "/workspace/app".to_string(),
            options: Some("ro".to_string()),
            name: Some("app".to_string()),
        }],
        ports: vec!["3000:3000".to_string()],
        workdir: "/workspace/app".to_string(),
        proxy,
    }
}

#[test]
fn run_args() {
    assert_eq!(
        sandbox(false, None).run_args("/home/ubuntu/.claude", Some("app")),
        [
            "run",
            "--rm",
            "-it",
            &format!("--label={IMAGE_LABEL}=dev"),
            "--name=app",
            "--workdir=/workspace/app",
            "-v",
            "/src/app:/workspace/app:ro",
            "-v",
            "/config/dev:/home/ubuntu/.claude",
            "-p",
            "3000:3000",
            "--env=COLORTERM",
            "--env=TERM_PROGRAM",
            "--env=TERM_PROGRAM_VERSION",
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
    let args = sandbox(true, Some(proxy)).run_args("/home/ubuntu/.claude", None);
    assert!(!args.contains(&"--rm".to_string()));
    let proxy_args = proxy::run_args();
    let start = args.iter().position(|arg| *arg == proxy_args[0]).unwrap();
    assert_eq!(args[start..start + proxy_args.len()], proxy_args);
    assert_eq!(
        args[start + proxy_args.len()..],
        ["--env=FOO=bar", "sndbx-dev"]
    );
}

#[test]
fn run_args_without_a_name() {
    let args = sandbox(false, None).run_args("/home/ubuntu/.claude", None);
    assert!(!args.iter().any(|arg| arg.starts_with("--name")));
}

#[test]
fn container_bases() {
    assert_eq!(container_base("hdf").as_deref(), Some("hdf"));
    assert_eq!(
        container_base("my_app.v2-x").as_deref(),
        Some("my_app-v2-x")
    );
    assert_eq!(
        container_base("kubernetes.io").as_deref(),
        Some("kubernetes-io")
    );
    assert_eq!(container_base("my  app").as_deref(), Some("my-app"));
    assert_eq!(container_base(".dotfiles").as_deref(), Some("dotfiles"));
    assert_eq!(container_base("_-x").as_deref(), Some("x"));
    assert_eq!(container_base("café").as_deref(), Some("caf"));
    assert_eq!(
        container_base("my app (old)").as_deref(),
        Some("my-app-old")
    );
    assert_eq!(container_base("проект-2").as_deref(), Some("2"));
    for unusable in ["", "проект", "...", "_-"] {
        assert_eq!(container_base(unusable), None, "{unusable}");
    }
}

#[test]
fn free_names() {
    let taken = |names: &[&str]| names.iter().map(ToString::to_string).collect();
    assert_eq!(free_name("hdf", &taken(&[])), "hdf");
    assert_eq!(free_name("hdf", &taken(&["other", "hdf-1"])), "hdf");
    assert_eq!(free_name("hdf", &taken(&["hdf"])), "hdf-1");
    assert_eq!(
        free_name("hdf", &taken(&["hdf", "hdf-1", "hdf-3"])),
        "hdf-2"
    );
    assert_eq!(free_name("foo-1", &taken(&["foo-1"])), "foo-1-1");
    assert_eq!(free_name("a", &taken(&[])), "a-1");
}

#[test]
fn name_args() {
    let args = |args: &[&str]| args.iter().map(ToString::to_string).collect::<Vec<_>>();
    assert!(has_name_arg(&args(&["--memory=8g", "--name", "custom"])));
    assert!(has_name_arg(&args(&["--name=custom"])));
    assert!(!has_name_arg(&args(&[
        "--memory=8g",
        "--env=NAME=x",
        "--names"
    ])));
}

#[test]
fn normalizes_ports() {
    assert_eq!(normalize_port("3000").unwrap(), "3000:3000");
    assert_eq!(normalize_port("53/udp").unwrap(), "53:53/udp");
    assert_eq!(normalize_port("8080:80").unwrap(), "8080:80");
    assert_eq!(
        normalize_port("127.0.0.1:3080:3080").unwrap(),
        "127.0.0.1:3080:3080"
    );
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
