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
                panic!("permission changes require no subprocess")
            })
            .unwrap();
    }
    session
}
#[test]
fn permission_numbers_and_symbolic_modes_match_tcl() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("file");
    fs::write(&file, "payload").unwrap();
    let mut session = session(true);
    let name = file.to_str().unwrap();
    for (value, expected) in [
        ("0755", 0o755),
        ("420", 0o644),
        ("0o600", 0o600),
        ("0x1ed", 0o755),
        ("0b110100000", 0o640),
        ("rwxr-x---", 0o750),
        ("u=rw,g=r,o=", 0o640),
        ("+x", 0o751),
        ("g-w,o=", 0o750),
        ("u", 0o750),
        ("", 0o750),
        ("a=rw,", 0o666),
    ] {
        assert_eq!(
            session
                .invoke("file", &["attributes", name, "-permissions", value])
                .unwrap(),
            ""
        );
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o7777,
            expected,
            "{value}"
        );
        assert_eq!(
            session.invoke("file", &["attributes", name, "-p"]).unwrap(),
            format!("0{expected:04o}")
        );
    }
    assert_eq!(fs::read_to_string(file).unwrap(), "payload");
}
#[test]
fn permission_changes_follow_links_and_queries_are_available_in_metadata() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("directory");
    fs::create_dir(&directory).unwrap();
    let link = root.path().join("link");
    symlink(&directory, &link).unwrap();
    session(true)
        .invoke(
            "file",
            &["attributes", link.to_str().unwrap(), "-permissions", "0750"],
        )
        .unwrap();
    assert_eq!(
        fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o750
    );
    assert_eq!(
        session(false)
            .invoke(
                "file",
                &["attributes", directory.to_str().unwrap(), "-permissions"]
            )
            .unwrap(),
        "040750"
    );
}
#[test]
fn invalid_modes_and_missing_files_are_catchable() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("file");
    fs::write(&file, "payload").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
    let mut session = session(true);
    session
        .invoke("set", &["path", file.to_str().unwrap()])
        .unwrap();
    for mode in ["u+X", "g=u", "u+r-w", "invalid", "rwxrwxrwq"] {
        session.invoke("set", &["mode", mode]).unwrap();
        assert_eq!(
            session
                .invoke(
                    "catch",
                    &["file attributes $path -permissions $mode", "message"]
                )
                .unwrap(),
            "1"
        );
        assert_eq!(session.scalar("errorCode").unwrap(), "TCL VALUE PERMISSION");
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
    session
        .invoke(
            "set",
            &["path", root.path().join("missing").to_str().unwrap()],
        )
        .unwrap();
    session
        .invoke("catch", &["file attributes $path -permissions 0755"])
        .unwrap();
    assert_eq!(
        session.scalar("errorCode").unwrap(),
        "POSIX ENOENT {no such file or directory}"
    );
}
#[test]
fn metadata_cannot_change_permissions_and_unsupported_attributes_are_explicit() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("file");
    fs::write(&file, "payload").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
    assert!(
        Session::open(
            "name app; version 1; catch {file attributes $path -permissions 0777}",
            &BTreeMap::from([("path".into(), file.to_str().unwrap().into())]),
            &PortSpec::named("app"),
            PortGroups::default()
        )
        .is_err()
    );
    assert_eq!(
        fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(
        session(false)
            .invoke("file", &["attributes", file.to_str().unwrap()])
            .is_err()
    );
    assert!(
        session(false)
            .invoke(
                "file",
                &["attributes", file.to_str().unwrap(), "-unsupported"]
            )
            .is_err()
    );
}
