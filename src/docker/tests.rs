use super::*;

#[test]
fn user_mappings() {
    assert!(!has_user_mapping(
        "[name=seccomp,profile=builtin name=cgroupns]"
    ));
    assert!(!has_user_mapping(""));
    assert!(has_user_mapping(
        "[name=seccomp,profile=builtin name=rootless name=cgroupns]"
    ));
    assert!(has_user_mapping(
        "[name=userns name=seccomp,profile=builtin]"
    ));
}

#[test]
fn network_containers() {
    let started = "2026-11-07T12:00:00.123456789Z";
    let container = |line: &str| parse_network_container(line, "sndbx-internal");

    assert_eq!(
        container(&format!(
            "/my-project {started} bridge=172.17.0.2 sndbx-internal=172.18.0.3"
        )),
        Some(NetworkContainer {
            name: "my-project".to_string(),
            ip: "172.18.0.3".to_string(),
            started: started.parse().unwrap(),
        })
    );
    assert_eq!(
        container(&format!("/other {started} bridge=172.17.0.2")),
        None
    );
    assert_eq!(
        container(&format!("/other {started} sndbx-internal=")),
        None
    );
    assert_eq!(
        container("/other yesterday sndbx-internal=172.18.0.3"),
        None
    );
    assert_eq!(container(""), None);
}
