use super::*;

fn new_entry(name: &str, keys: &str) -> Result<NewEntry> {
    NewEntry::new(name, keys, Path::new("sndbx.yml"))
}

fn added_with(content: &str, name: &str, keys: &str) -> Option<String> {
    addition(
        &parse(content).unwrap(),
        content,
        &new_entry(name, keys).unwrap(),
    )
    .map(|addition| format!("{content}{addition}"))
}

fn added(content: &str, name: &str) -> Option<String> {
    added_with(content, name, "")
}

fn error(name: &str, keys: &str) -> String {
    format!("{:#}", new_entry(name, keys).err().unwrap())
}

#[test]
fn appends_the_keys_under_the_name() {
    let keys = "# web UI\nports: ['127.0.0.1:3080:3080']\n\nproxy: false\n\n";
    assert_eq!(
        added_with("images:\n    - name: a\n", "b", keys).unwrap(),
        "images:\n    - name: a\n    - name: 'b'\n      # web UI\n      ports: ['127.0.0.1:3080:3080']\n\n      proxy: false\n"
    );
    assert_eq!(
        added_with("", "b", "docker_args:\n  - x\n").unwrap(),
        "images:\n  - name: 'b'\n    docker_args:\n      - x\n"
    );
    assert_eq!(
        added_with("images:\n  - name: a\n", "b", "# nothing\n").unwrap(),
        "images:\n  - name: a\n  - name: 'b'\n    # nothing\n"
    );
}

#[test]
fn checks_the_keys() {
    assert_eq!(
        error("b", "name: c\n"),
        "invalid sndbx.yml: name is the image directory's"
    );
    assert!(error("b", "port: '1'\n").contains("unknown field `port`"));
    assert!(error("b", "ports: [1]\n").contains("expected a string"));
    assert!(error("b", "- x\n").starts_with("invalid sndbx.yml: "));
    for keys in ["{ports: ['1']}\n", "user: x\n---\ntag: y\n"] {
        assert_eq!(
            error("b", keys),
            "invalid sndbx.yml: write each key at the start of a line"
        );
    }
    assert!(!new_entry("b", "").unwrap().has_config_target());
    assert!(
        new_entry("b", "config_target: .config/b")
            .unwrap()
            .has_config_target()
    );
}

#[test]
fn appends_to_the_images_list() {
    assert_eq!(
        added("images:\n  - name: a\n", "b").unwrap(),
        "images:\n  - name: a\n  - name: 'b'\n"
    );
    assert_eq!(
        added("images:\n- name: a\n  user: x", "b").unwrap(),
        "images:\n- name: a\n  user: x\n- name: 'b'\n"
    );
    assert_eq!(
        added("tag_prefix: x\n", "b").unwrap(),
        "tag_prefix: x\nimages:\n  - name: 'b'\n"
    );
    assert_eq!(
        added(
            "images:\n    - name: a\n      docker_args:\n        - x\n",
            "b"
        )
        .unwrap(),
        "images:\n    - name: a\n      docker_args:\n        - x\n    - name: 'b'\n"
    );
    assert_eq!(added("", "b").unwrap(), "images:\n  - name: 'b'\n");
    assert_eq!(added("images: []\n# a\n", "b"), None);
    assert_eq!(
        added("images:\n  - name: a\ndocker_args:\n  - x\n", "b"),
        None
    );
    assert_eq!(added("images: ~\n", "b"), None);
    assert_eq!(added("images:\n  - name: a\n---\nx: 1\n", "b"), None);
}

#[test]
fn appends_to_the_example() {
    let example = added(EXAMPLE, "b").unwrap();
    assert!(example.ends_with("\n  - name: 'b'\n"), "{example}");
}

#[test]
fn quotes_names() {
    for name in ["true", "123", "null", "it's"] {
        let content = added("images:\n  - name: a\n", name).unwrap();
        assert_eq!(parse(&content).unwrap().images.unwrap()[1].name, name);
    }
}

fn commented(content: &str, index: usize) -> Option<String> {
    commented_out(&parse(content).unwrap(), content, index, "# Removed:")
}

#[test]
fn comments_out_an_entry() {
    assert_eq!(
        commented("images:\n  - name: a\n    ports: ['1']\n  - name: b\n", 0).unwrap(),
        "images:\n  # Removed:\n  # - name: a\n  #   ports: ['1']\n  - name: b\n"
    );
    assert_eq!(
        commented(
            "images:\n- name: a\n- user: x\n  name: b\ndocker_args: [y]\n",
            1
        )
        .unwrap(),
        "images:\n- name: a\n# Removed:\n# - user: x\n#   name: b\ndocker_args: [y]\n"
    );
}

#[test]
fn comments_out_an_entry_with_a_list() {
    let content = "images:\n  - name: a\n    docker_args:\n      - x\n\n    # mine\n    ports: ['1']\n  - name: b\n";
    assert_eq!(
        commented(content, 0).unwrap(),
        "images:\n  # Removed:\n  # - name: a\n  #   docker_args:\n  #     - x\n\n    # mine\n  #   ports: ['1']\n  - name: b\n"
    );
    assert_eq!(
        commented(content, 1).unwrap(),
        "images:\n  - name: a\n    docker_args:\n      - x\n\n    # mine\n    ports: ['1']\n  # Removed:\n  # - name: b\n"
    );
}

#[test]
fn comments_out_the_last_entry() {
    assert_eq!(
        commented("images:\n  - name: a\n\n# end", 0).unwrap(),
        "images:\n  # Removed:\n  # - name: a\n\n# end"
    );
    assert_eq!(
        commented("images:\n  - name: a", 0).unwrap(),
        "images:\n  # Removed:\n  # - name: a\n"
    );
}

#[test]
fn comments_out_only_whole_entries() {
    assert_eq!(commented("images: [{name: a}]\n", 0), None);
    assert_eq!(
        commented("docker_args:\n  - x\nimages: [{name: a}]\n", 0),
        None
    );
}
