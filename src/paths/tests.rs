use super::*;

#[test]
fn expand_path_normalizes() {
    assert_eq!(expand_path("/a/./b/../c/", None), Path::new("/a/c"));
    assert_eq!(expand_path("//a/b", None), Path::new("/a/b"));
    assert_eq!(expand_path("/..", None), Path::new("/"));
    assert_eq!(expand_path("b", Some(Path::new("/a"))), Path::new("/a/b"));
    assert_eq!(expand_path("/b", Some(Path::new("/a"))), Path::new("/b"));
    assert_eq!(expand_path("~/x/..", None), home());
    assert_eq!(expand_path("~x", Some(Path::new("/a"))), Path::new("/a/~x"));
}

#[test]
fn join_uses_one_slash() {
    assert_eq!(
        join(&["/home", "ubuntu", ".claude"]),
        "/home/ubuntu/.claude"
    );
    assert_eq!(join(&["/home/", "/ubuntu/", "/x"]), "/home/ubuntu/x");
    assert_eq!(join(&["/workspace", ""]), "/workspace/");
}

#[test]
fn shell_join_escapes() {
    let words = ["docker", "my dir", "", "a\nb", "x=$y", "/a:b@c,d+e"];
    assert_eq!(
        shell_join(&words),
        "docker my\\ dir '' a'\n'b x\\=\\$y /a:b@c,d+e"
    );
}
