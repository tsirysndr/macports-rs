use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

fn session(execution: bool) -> Session {
    let mut session = Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    if execution {
        session
            .enable_system(&BTreeMap::new(), |_| {
                panic!("mkdir must not launch a subprocess")
            })
            .unwrap();
        session.initialize_creation_mask(0o027).unwrap();
    }
    session
}

#[test]
fn mkdir_creates_multiple_trees_with_session_mask_and_preserves_existing_modes() {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
    let first = root.path().join("source with spaces/deep");
    let second = root.path().join("stage/usr/bin");
    let mut session = session(true);
    assert_eq!(session.invoke("file", &["mkdir"]).unwrap(), "");
    session
        .invoke(
            "file",
            &["mkdir", first.to_str().unwrap(), second.to_str().unwrap()],
        )
        .unwrap();
    for path in [&first, &second, &root.path().join("stage/usr")] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o750
        );
    }
    session.invoke("umask", &["000"]).unwrap();
    session
        .invoke("file", &["mkdir", first.to_str().unwrap()])
        .unwrap();
    assert_eq!(
        fs::metadata(&first).unwrap().permissions().mode() & 0o777,
        0o750
    );
    assert_eq!(
        fs::metadata(root.path()).unwrap().permissions().mode() & 0o777,
        0o755
    );
    let open = root.path().join("open");
    session
        .invoke("file", &["mkdir", open.to_str().unwrap()])
        .unwrap();
    assert_eq!(
        fs::metadata(open).unwrap().permissions().mode() & 0o777,
        0o777
    );
}

#[test]
fn mkdir_follows_directory_links_and_does_not_elide_parent_components() {
    let root = tempfile::tempdir().unwrap();
    symlink(root.path(), root.path().join("link")).unwrap();
    let target = root.path().join("link/created/../sibling");
    session(true)
        .invoke("file", &["mkdir", target.to_str().unwrap()])
        .unwrap();
    assert!(root.path().join("created").is_dir());
    assert!(root.path().join("sibling").is_dir());
}

#[test]
fn mkdir_errors_are_catchable_report_the_failing_component_and_stop_remaining_paths() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("file");
    fs::write(&file, "untouched").unwrap();
    let target = file.join("child");
    let last = root.path().join("last");
    let mut session = session(true);
    session
        .invoke("set", &["target", target.to_str().unwrap()])
        .unwrap();
    session
        .invoke("catch", &["file mkdir $target", "msg"])
        .unwrap();
    assert_eq!(
        session.scalar("errorCode").unwrap(),
        "POSIX EEXIST {file already exists}"
    );
    assert_eq!(
        session.scalar("msg").unwrap(),
        format!(
            "can't create directory \"{}\": file already exists",
            file.display()
        )
    );
    assert!(
        session
            .invoke(
                "file",
                &["mkdir", target.to_str().unwrap(), last.to_str().unwrap()]
            )
            .is_err()
    );
    assert!(!last.exists());
    assert_eq!(fs::read_to_string(file).unwrap(), "untouched");
    session.invoke("catch", &["file mkdir {}"]).unwrap();
    assert_eq!(
        session.scalar("errorCode").unwrap(),
        "POSIX ENOENT {no such file or directory}"
    );
    symlink("missing", root.path().join("broken")).unwrap();
    assert!(
        session
            .invoke(
                "file",
                &["mkdir", root.path().join("broken").to_str().unwrap()]
            )
            .is_err()
    );
    // Ordinary errors do not poison the execution session.
    session
        .invoke("file", &["mkdir", last.to_str().unwrap()])
        .unwrap();
    assert!(last.is_dir());
}

#[test]
fn mkdir_remains_unavailable_during_metadata_evaluation_even_when_caught() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("forbidden");
    let variables = BTreeMap::from([("target".into(), target.to_string_lossy().into_owned())]);
    let result = Session::open(
        "name app; version 1; catch {file mkdir $target}",
        &variables,
        &PortSpec::named("app"),
        PortGroups::default(),
    );
    assert!(result.is_err());
    assert!(!target.exists());
}
