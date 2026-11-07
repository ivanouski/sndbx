use super::*;

#[test]
fn hosts() {
    let content = "# models\n.openai.com\n\n  pypi.org  # packages\n.openai.com\n";
    assert_eq!(
        parse(content, Path::new("x")).unwrap(),
        [".openai.com", "pypi.org"]
    );
    assert_eq!(parse("", Path::new("x")).unwrap(), Vec::<String>::new());
}

#[test]
fn drops_hosts_a_domain_covers() {
    let content =
        "api.github.com\n.x.github.com\ngithub.com\n.github.com\nmygithub.com\npypi.org\n";
    assert_eq!(
        parse(content, Path::new("x")).unwrap(),
        [".github.com", "mygithub.com", "pypi.org"]
    );
}

#[test]
fn invalid_hosts() {
    let message = parse("a.com\nopen ai.com\n", Path::new("x"))
        .err()
        .unwrap()
        .to_string();
    assert_eq!(
        message,
        "invalid x line 2: 'open ai.com' is not a host name like .openai.com or pypi.org"
    );
    for invalid in [
        "Example.com",
        "a..com",
        "..a",
        "-a.com",
        "a-.com",
        "http_access",
        "a.com/x",
        "*.a.com",
    ] {
        assert!(!is_valid_host(invalid), "{invalid}");
    }
    for valid in ["localhost", "a.com", ".a.com", "x-1.a2.com", "1.2.3.4"] {
        assert!(is_valid_host(valid), "{valid}");
    }
}
