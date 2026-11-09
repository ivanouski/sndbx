use super::*;

#[test]
fn tag_prefixes() {
    for valid in ["sndbx", "my.org/sndbx", "a-b_c", "x1"] {
        assert!(is_valid_tag_prefix(valid), "{valid}");
    }
    for invalid in ["", "Not Valid", "a--b", "-a", "a/", "Sndbx"] {
        assert!(!is_valid_tag_prefix(invalid), "{invalid}");
    }
}

#[test]
fn example_parses() {
    let example = parse(EXAMPLE).unwrap();
    assert_eq!(
        example.docker_args,
        ["--security-opt=no-new-privileges", "--pids-limit=4096"]
    );
    assert_eq!(example.images.unwrap()[0].name, "claude");
}

#[test]
fn empty_settings() {
    for empty in ["", "# comment\n", "~\n", "images: ~\n"] {
        assert!(parse(empty).unwrap().images.is_none(), "{empty:?}");
    }
}

#[test]
fn only_the_first_document() {
    assert_eq!(
        parse("images: []\n---\nimages: 1\n").unwrap().images,
        Some(vec![])
    );
}

#[test]
fn duplicate_keys() {
    let message = parse("images: []\nimages: []\n").err().unwrap().to_string();
    assert!(message.contains("duplicate field `images`"), "{message}");
}

#[test]
fn strings_and_lists() {
    let image = parse("images:\n  - name: a\n    ports: '3000'\n    docker_args: [x, y]\n")
        .unwrap()
        .images
        .unwrap()
        .remove(0);
    assert_eq!(image.ports, ["3000"]);
    assert_eq!(image.docker_args, ["x", "y"]);
}

fn expand(args: &[&str]) -> Vec<String> {
    let args: Vec<String> = args.iter().map(ToString::to_string).collect();
    expand_home_in_args(&args, Path::new("/home/me"))
}

#[test]
fn expands_home_in_path_flags() {
    for (arg, expanded) in [
        ("--env-file=~/a.env", "--env-file=/home/me/a.env"),
        ("--label-file=~/labels", "--label-file=/home/me/labels"),
        ("--volume=~/a:/a:ro", "--volume=/home/me/a:/a:ro"),
        ("-v=~/a:/a", "-v=/home/me/a:/a"),
        (
            "--mount=type=bind,src=~/a,dst=/a",
            "--mount=type=bind,src=/home/me/a,dst=/a",
        ),
        (
            "--mount=type=bind,source=~/a,target=/a,readonly",
            "--mount=type=bind,source=/home/me/a,target=/a,readonly",
        ),
    ] {
        assert_eq!(expand(&[arg]), [expanded], "{arg}");
    }
    assert_eq!(
        expand(&[
            "--env-file",
            "~/a.env",
            "-v",
            "~/b:/b",
            "--mount",
            "src=~/c,dst=/c"
        ]),
        [
            "--env-file",
            "/home/me/a.env",
            "-v",
            "/home/me/b:/b",
            "--mount",
            "src=/home/me/c,dst=/c"
        ]
    );
}

#[test]
fn leaves_other_args_be() {
    for args in [
        &["-e", "FOO=~/x"][..],
        &["-e=FOO=~/x"],
        &["--env-file=/a.env", "--env-file", "a.env"],
        &["--env-file=~user/a.env", "-v", "~:/home"],
        &["-v", "~/a"],
        &["--volume=~/a"],
        &["--mount=type=bind,dst=~/a,src=/a"],
        &["~/a", "--memory=8g"],
        &["--env-file"],
    ] {
        assert_eq!(expand(args), args, "{args:?}");
    }
}

#[test]
fn proxies() {
    let proxy = |content| parse(content).unwrap().proxy;
    assert_eq!(proxy("images: []\n"), None);
    assert_eq!(proxy("proxy: ~\n"), None);
    assert_eq!(
        proxy("proxy:\n  image: squid\n"),
        Some(Proxy {
            image: "squid".to_string(),
            allow: Allow::Listed
        })
    );
    assert_eq!(
        proxy("proxy:\n  image: squid\n  allow: all\n")
            .unwrap()
            .allow,
        Allow::All
    );
    assert_eq!(
        proxy("proxy:\n  image: squid\n  allow:\n").unwrap().allow,
        Allow::Listed
    );
    let message = parse("proxy:\n  image: squid\n  allow: any\n")
        .err()
        .unwrap()
        .to_string();
    assert!(
        message.contains("proxy.allow: unknown variant `any`, expected `listed` or `all`"),
        "{message}"
    );
    let message = parse("proxy:\n  allow: all\n").err().unwrap().to_string();
    assert!(
        message.contains("proxy: missing field `image`"),
        "{message}"
    );
}

#[test]
fn unknown_keys() {
    let message = parse("image: []\n").err().unwrap().to_string();
    assert!(message.contains("unknown field `image`"), "{message}");
    let message = parse("images:\n  - name: a\n    port: '3000'\n")
        .err()
        .unwrap()
        .to_string();
    assert!(
        message.contains("images[0]: unknown field `port`"),
        "{message}"
    );
}

#[test]
fn only_strings() {
    for invalid in [
        "images:\n  - name: a\n    ports: [3000]\n",
        "images:\n  - name: a\n    docker_args: 8\n",
        "images:\n  - name: a\n    user: 5\n",
        "images:\n  - name: a\n    config_dir: 42\n",
        "images:\n  - name: true\n",
        "tag_prefix: 1\n",
        "docker_args: [8]\n",
        "proxy:\n  image: 1\n",
    ] {
        let message = parse(invalid).err().unwrap().to_string();
        assert!(
            message.contains("expected a string"),
            "{invalid:?}: {message}"
        );
    }
}

#[test]
fn config_targets() {
    for valid in [
        ".claude",
        ".config/opencode",
        "/opt/claude",
        "a..b/.x",
        "~foo",
    ] {
        assert_eq!(config_target_problem(valid), None, "{valid}");
    }
    for invalid in [
        "",
        "/",
        "//",
        "/.",
        "~",
        "~/.claude",
        "../x",
        ".claude/../..",
        "/opt/../etc",
    ] {
        assert!(config_target_problem(invalid).is_some(), "{invalid:?}");
    }
}
