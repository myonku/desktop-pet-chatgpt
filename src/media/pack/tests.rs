use super::*;

#[test]
fn rejects_absolute_and_traversing_paths_before_filesystem_access() {
    for relative in [
        "../outside",
        "sub/../outside",
        "/root",
        "C:/file",
        r"sub\file",
        "file:stream",
    ] {
        let error = contained_path(Path::new("nonexistent-pack"), relative).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{relative}");
    }
}
