use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{MetadataExt, symlink},
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
            .enable_system(&BTreeMap::new(), |_| panic!("links require no subprocess"))
            .unwrap();
    }
    session
}
#[test]
fn symbolic_links_resolve_relative_targets_from_parent_and_queries_work_in_metadata() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    fs::write(&target, "content").unwrap();
    let name = root.path().join("link");
    let mut execution = session(true);
    assert_eq!(
        execution
            .invoke("file", &["link", name.to_str().unwrap(), "target"])
            .unwrap(),
        "target"
    );
    assert_eq!(fs::read_link(&name).unwrap().to_str().unwrap(), "target");
    assert_eq!(fs::read_to_string(&name).unwrap(), "content");
    assert_eq!(
        session(false)
            .invoke("file", &["link", name.to_str().unwrap()])
            .unwrap(),
        "target"
    );
    let directory = root.path().join("directory");
    fs::create_dir(&directory).unwrap();
    let dirlink = root.path().join("dirlink");
    execution
        .invoke(
            "file",
            &["link", "-s", dirlink.to_str().unwrap(), "directory"],
        )
        .unwrap();
    assert!(dirlink.is_dir());
}
#[test]
fn hard_links_share_inode_and_return_target() {
    let root = tempfile::tempdir_in(".").unwrap();
    let target = root.path().join("source");
    fs::write(&target, "content").unwrap();
    let name = root.path().join("hard");
    let mut execution = session(true);
    assert_eq!(
        execution
            .invoke(
                "file",
                &[
                    "link",
                    "-h",
                    name.to_str().unwrap(),
                    target.to_str().unwrap()
                ]
            )
            .unwrap(),
        target.to_str().unwrap()
    );
    assert_eq!(
        fs::metadata(&target).unwrap().ino(),
        fs::metadata(&name).unwrap().ino()
    );
    fs::write(&name, "changed").unwrap();
    assert_eq!(fs::read_to_string(target).unwrap(), "changed");
    assert!(
        execution
            .invoke("file", &["link", name.to_str().unwrap()])
            .is_err()
    );
}
#[test]
fn missing_targets_and_existing_destinations_produce_catchable_errors() {
    let root = tempfile::tempdir().unwrap();
    let name = root.path().join("link");
    let target = root.path().join("target");
    let mut execution = session(true);
    execution
        .invoke("set", &["name", name.to_str().unwrap()])
        .unwrap();
    assert_eq!(
        execution
            .invoke("catch", &["file link $name missing", "message"])
            .unwrap(),
        "1"
    );
    assert_eq!(
        execution.scalar("errorCode").unwrap(),
        "POSIX ENOENT {no such file or directory}"
    );
    assert!(
        execution
            .scalar("message")
            .unwrap()
            .contains("target \"missing\" doesn't exist")
    );
    assert!(fs::symlink_metadata(&name).is_err());
    fs::write(&target, "target").unwrap();
    fs::write(&name, "keep").unwrap();
    execution
        .invoke("catch", &["file link $name target", "message"])
        .unwrap();
    assert_eq!(
        execution.scalar("errorCode").unwrap(),
        "POSIX EEXIST {file already exists}"
    );
    assert!(
        execution
            .scalar("message")
            .unwrap()
            .contains("that path already exists")
    );
    assert_eq!(fs::read_to_string(&name).unwrap(), "keep");
    let missing_parent = root.path().join("missing/link");
    let error = execution
        .invoke(
            "file",
            &[
                "link",
                missing_parent.to_str().unwrap(),
                target.to_str().unwrap(),
            ],
        )
        .unwrap_err();
    assert!(error.to_string().contains("no such file or directory"));
    assert!(
        execution
            .invoke("file", &["link", "-", name.to_str().unwrap(), "target"])
            .unwrap_err()
            .to_string()
            .contains("ambiguous option")
    );
    assert!(
        execution
            .invoke(
                "file",
                &["link", "-invalid", name.to_str().unwrap(), "target"]
            )
            .unwrap_err()
            .to_string()
            .contains("bad option")
    );
}
#[test]
fn metadata_can_read_dangling_links_but_cannot_create_links_even_if_caught() {
    let root = tempfile::tempdir().unwrap();
    let link = root.path().join("dangling");
    symlink("missing", &link).unwrap();
    assert_eq!(
        session(false)
            .invoke("file", &["link", link.to_str().unwrap()])
            .unwrap(),
        "missing"
    );
    let name = root.path().join("new-link");
    let variables = BTreeMap::from([
        ("name".into(), name.to_str().unwrap().into()),
        ("target".into(), root.path().to_str().unwrap().into()),
    ]);
    assert!(
        Session::open(
            "name app; version 1; catch {file link $name $target}",
            &variables,
            &PortSpec::named("app"),
            PortGroups::default()
        )
        .is_err()
    );
    assert!(fs::symlink_metadata(name).is_err());
}
