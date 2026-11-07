use super::*;

#[test]
fn finds_the_base_image() {
    assert!(is_from_base(
        "ARG TAG_PREFIX=sndbx\nFROM ${TAG_PREFIX}:base\n"
    ));
    assert!(is_from_base("FROM\t${TAG_PREFIX}:base AS build\r\n"));
    assert!(is_from_base("FROM ${TAG_PREFIX}:base"));
    assert!(is_from_base("FROM  ${TAG_PREFIX}:base  "));
    assert!(!is_from_base("FROM ubuntu\n"));
    assert!(!is_from_base("FROM ${TAG_PREFIX}:base2\n"));
    assert!(!is_from_base("FROM${TAG_PREFIX}:base\n"));
    assert!(!is_from_base(" FROM ${TAG_PREFIX}:base\n"));
    assert!(!is_from_base("# FROM ${TAG_PREFIX}:base\n"));
}

#[test]
fn the_repo_images_have_valid_entry_files() {
    let builder = Builder::new(&Settings::default());
    for name in builder.image_names().unwrap() {
        if let Err(e) = builder.new_entry(&name) {
            panic!("{e:#}");
        }
    }
    assert!(builder.new_entry("opencode").unwrap().has_config_target());
}
