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
