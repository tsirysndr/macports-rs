use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};
fn session(root: &Path, execution: bool) -> Session {
    let mut s = Session::open(
        "name app; version 1",
        &BTreeMap::from([("destroot".into(), root.to_str().unwrap().into())]),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    if execution {
        s.enable_system(&BTreeMap::new(), |_| {
            panic!("pruning must not spawn a shell")
        })
        .unwrap();
    }
    s
}
#[test]
fn keeps_requested_directories_with_exact_modes_and_prunes_other_empty_trees() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("stage");
    fs::create_dir_all(root.join("empty/nested")).unwrap();
    let mut s = session(&root, true);
    let already_staged = root.join("opt/local/var/already");
    s.invoke(
        "destroot.keepdirs",
        &[
            "/opt/local/var/absolute",
            "opt/local/var/relative",
            already_staged.to_str().unwrap(),
        ],
    )
    .unwrap();
    let result = s.prune_destroot_directories().unwrap();
    assert_eq!(result.created_markers, 3);
    assert_eq!(result.removed_directories, 2);
    assert!(!root.join("empty").exists());
    for leaf in ["absolute", "relative", "already"] {
        let directory = root.join("opt/local/var").join(leaf);
        let marker = directory.join(".turd_app");
        assert_eq!(
            fs::metadata(directory).unwrap().permissions().mode() & 0o7777,
            0o755
        );
        assert_eq!(
            fs::metadata(&marker).unwrap().permissions().mode() & 0o7777,
            0o644
        );
        assert_eq!(fs::read(marker).unwrap(), b"");
    }
    assert_eq!(s.prune_destroot_directories().unwrap().created_markers, 0);
}
#[test]
fn pruning_an_empty_stage_removes_it_and_reports_staging_failure() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("stage");
    fs::create_dir_all(root.join("empty/deep")).unwrap();
    let error = session(&root, true)
        .prune_destroot_directories()
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Staging app into destroot failed")
    );
    assert!(!root.exists());
}
#[test]
fn existing_markers_files_and_directory_symlinks_are_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("stage");
    let outside = temp.path().join("outside");
    fs::create_dir_all(root.join("keep")).unwrap();
    fs::create_dir_all(outside.join("empty")).unwrap();
    let marker = root.join("keep/.turd_app");
    fs::write(&marker, "preserve existing content").unwrap();
    fs::set_permissions(&marker, fs::Permissions::from_mode(0o600)).unwrap();
    symlink(&outside, root.join("link")).unwrap();
    symlink("missing", root.join("dangling")).unwrap();
    let mut s = session(&root, true);
    s.invoke("destroot.keepdirs", &["keep"]).unwrap();
    assert_eq!(s.prune_destroot_directories().unwrap().created_markers, 0);
    assert_eq!(
        fs::read_to_string(&marker).unwrap(),
        "preserve existing content"
    );
    assert_eq!(
        fs::metadata(marker).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(outside.join("empty").is_dir());
    assert!(fs::symlink_metadata(root.join("dangling")).is_ok());
}
#[test]
fn parent_references_are_jailed_and_outward_directory_links_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("stage");
    let outside = temp.path().join("outside");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&outside).unwrap();
    let mut s = session(&root, true);
    s.invoke("destroot.keepdirs", &["../../keep"]).unwrap();
    s.prune_destroot_directories().unwrap();
    assert!(root.join("keep/.turd_app").exists());
    symlink(&outside, root.join("escape")).unwrap();
    s.invoke("destroot.keepdirs", &["escape/child"]).unwrap();
    assert!(
        s.prune_destroot_directories()
            .unwrap_err()
            .to_string()
            .contains("outside destroot")
    );
    assert!(!outside.join("child").exists());
}
#[test]
fn dangling_marker_is_replaced_and_metadata_cannot_prune() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("stage");
    fs::create_dir_all(root.join("keep")).unwrap();
    let outside = temp.path().join("absent");
    symlink(&outside, root.join("keep/.turd_app")).unwrap();
    assert!(session(&root, false).prune_destroot_directories().is_err());
    assert!(root.is_dir());
    let mut s = session(&root, true);
    s.invoke("destroot.keepdirs", &["keep"]).unwrap();
    assert_eq!(s.prune_destroot_directories().unwrap().created_markers, 1);
    assert!(
        fs::symlink_metadata(root.join("keep/.turd_app"))
            .unwrap()
            .is_file()
    );
    assert!(!outside.exists());
}
