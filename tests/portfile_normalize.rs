use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{collections::BTreeMap, fs, os::unix::fs::symlink, path::Path};
fn normalize(path: &str) -> String {
    let mut session = Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    session.invoke("file", &["normalize", path]).unwrap()
}
#[test]
fn normalizes_relative_and_missing_paths_without_creating_them() {
    assert_eq!(normalize(""), "");
    assert_eq!(normalize("/../../."), "/");
    let cwd = fs::canonicalize(".").unwrap();
    assert_eq!(
        normalize("./missing-parent/../missing-child"),
        cwd.join("missing-child").to_str().unwrap()
    );
    assert_eq!(
        normalize("~literal/file"),
        cwd.join("~literal/file").to_str().unwrap()
    );
    let root = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(root.path()).unwrap();
    assert_eq!(
        normalize(&format!("{}/missing//a/../b/", root.display())),
        root.join("missing/b").to_str().unwrap()
    );
    assert!(!root.join("missing").exists());
}
#[test]
fn resolves_existing_parent_links_but_preserves_the_final_link() {
    let temporary = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temporary.path()).unwrap();
    fs::create_dir_all(root.join("actual/deep")).unwrap();
    symlink("actual/deep", root.join("alias")).unwrap();
    assert_eq!(
        normalize(root.join("alias/child").to_str().unwrap()),
        root.join("actual/deep/child").to_str().unwrap()
    );
    assert_eq!(
        normalize(root.join("alias").to_str().unwrap()),
        root.join("alias").to_str().unwrap()
    );
    assert_eq!(
        normalize(root.join("alias/.").to_str().unwrap()),
        root.join("alias").to_str().unwrap()
    );
    assert_eq!(
        normalize(root.join("alias/../other").to_str().unwrap()),
        root.join("actual/other").to_str().unwrap()
    );
    symlink(root.join("actual/deep"), root.join("absolute")).unwrap();
    assert_eq!(
        normalize(root.join("absolute/../other").to_str().unwrap()),
        root.join("actual/other").to_str().unwrap()
    );
}
#[test]
fn dangling_links_and_loops_do_not_require_existing_targets_or_hang() {
    let temporary = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temporary.path()).unwrap();
    symlink("missing/deep", root.join("broken")).unwrap();
    assert_eq!(
        normalize(root.join("broken/child").to_str().unwrap()),
        root.join("broken/child").to_str().unwrap()
    );
    assert_eq!(
        normalize(root.join("broken/../other").to_str().unwrap()),
        root.join("missing/other").to_str().unwrap()
    );
    symlink("cycle", root.join("cycle")).unwrap();
    assert_eq!(
        normalize(root.join("cycle/child").to_str().unwrap()),
        root.join("cycle/child").to_str().unwrap()
    );
    assert_eq!(
        normalize(root.join("cycle/../other").to_str().unwrap()),
        root.join("other").to_str().unwrap()
    );
}
#[test]
fn normalization_is_available_during_metadata_evaluation() {
    let source = "name app; version 1; description [file normalize ./missing/../source]";
    let session = Session::open(
        source,
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    let mut session = session;
    let result = session.scalar("description").unwrap();
    assert!(Path::new(&result).is_absolute());
    assert_eq!(
        result,
        fs::canonicalize(".")
            .unwrap()
            .join("source")
            .to_str()
            .unwrap()
    );
}
