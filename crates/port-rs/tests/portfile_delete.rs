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
fn session() -> Session {
    let mut session = Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    session
        .enable_system(&BTreeMap::new(), |_| panic!("no subprocess required"))
        .unwrap();
    session
}
fn delete(session: &mut Session, path: &Path, force: bool) -> anyhow::Result<String> {
    let mut args = vec!["delete"];
    if force {
        args.push("-force");
    }
    args.extend(["--", path.to_str().unwrap()]);
    session.invoke("file", &args)
}
#[test]
fn deletes_files_empty_directories_and_missing_paths() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("-file with spaces");
    let dir = root.path().join("empty");
    fs::write(&file, "payload").unwrap();
    fs::create_dir(&dir).unwrap();
    let mut session = session();
    session
        .invoke(
            "file",
            &["delete", file.to_str().unwrap(), dir.to_str().unwrap()],
        )
        .unwrap();
    assert!(!file.exists());
    assert!(!dir.exists());
    delete(&mut session, &file, false).unwrap();
    session.invoke("file", &["delete", ""]).unwrap();
    for args in [
        vec!["delete"],
        vec!["delete", "-force", "-force"],
        vec!["delete", "--"],
    ] {
        assert_eq!(session.invoke("file", &args).unwrap(), "");
    }
    assert!(
        session
            .invoke("file", &["delete", "-f"])
            .unwrap_err()
            .to_string()
            .contains("bad option")
    );
}
#[test]
fn recursive_deletion_preserves_targets_of_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let outside = root.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), "safe").unwrap();
    let tree = root.path().join("tree");
    fs::create_dir_all(tree.join("deep/child")).unwrap();
    fs::write(tree.join("deep/child/file"), "remove").unwrap();
    symlink(&outside, tree.join("external")).unwrap();
    symlink("missing", tree.join("broken")).unwrap();
    symlink(".", tree.join("cycle")).unwrap();
    let top_link = root.path().join("link");
    symlink(&outside, &top_link).unwrap();
    let mut session = session();
    delete(&mut session, &top_link, true).unwrap();
    assert!(fs::symlink_metadata(&top_link).is_err());
    fs::set_permissions(&tree, fs::Permissions::from_mode(0o500)).unwrap();
    delete(&mut session, &tree, true).unwrap();
    assert!(!tree.exists());
    assert_eq!(fs::read_to_string(outside.join("keep")).unwrap(), "safe");
}
#[test]
fn nonempty_directory_errors_are_catchable_and_stop_later_operands() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("dir");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("keep"), "keep").unwrap();
    let later = root.path().join("later");
    fs::write(&later, "keep").unwrap();
    let mut session = session();
    session
        .invoke("set", &["target", dir.to_str().unwrap()])
        .unwrap();
    assert_eq!(
        session
            .invoke("catch", &["file delete $target", "msg"])
            .unwrap(),
        "1"
    );
    assert_eq!(
        session.scalar("errorCode").unwrap(),
        "POSIX EEXIST {file already exists}"
    );
    assert!(
        session
            .scalar("msg")
            .unwrap()
            .contains("directory not empty")
    );
    assert!(
        session
            .invoke(
                "file",
                &["delete", dir.to_str().unwrap(), later.to_str().unwrap()]
            )
            .is_err()
    );
    assert!(later.exists());
    assert!(dir.join("keep").exists());
    delete(&mut session, &dir, true).unwrap();
}
#[test]
fn metadata_cannot_delete_even_when_error_is_caught() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("keep");
    fs::write(&file, "keep").unwrap();
    let result = Session::open(
        "name app; version 1; catch {file delete -force $target}",
        &BTreeMap::from([("target".into(), file.to_str().unwrap().into())]),
        &PortSpec::named("app"),
        PortGroups::default(),
    );
    assert!(result.is_err());
    assert!(file.exists());
}
#[test]
fn recursive_failure_restores_root_permissions_and_preserves_nested_permissions() {
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let tree = root.path().join("tree");
    let locked = tree.join("locked");
    fs::create_dir_all(&locked).unwrap();
    fs::write(locked.join("keep"), "keep").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    fs::set_permissions(&tree, fs::Permissions::from_mode(0o500)).unwrap();
    let result = delete(&mut session(), &tree, true);
    let tree_mode = fs::metadata(&tree).unwrap().permissions().mode() & 0o777;
    let nested_mode = fs::metadata(&locked).unwrap().permissions().mode() & 0o777;
    fs::set_permissions(&tree, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(result.unwrap_err().to_string().contains("locked"));
    assert_eq!(tree_mode, 0o500);
    assert_eq!(nested_mode, 0o000);
}
