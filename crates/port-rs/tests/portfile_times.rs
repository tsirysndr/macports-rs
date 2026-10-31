use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{MetadataExt, symlink},
    time::{Duration, UNIX_EPOCH},
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
        .enable_system(&BTreeMap::new(), |_| {
            panic!("timestamp changes must be native")
        })
        .unwrap();
    session
}
#[test]
fn setters_preserve_other_seconds_discard_nanoseconds_and_return_stored_value() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    let file = fs::File::create(&path).unwrap();
    file.set_times(
        fs::FileTimes::new()
            .set_accessed(UNIX_EPOCH + Duration::new(1_600_000_000, 123_456_789))
            .set_modified(UNIX_EPOCH + Duration::new(1_600_000_100, 987_654_321)),
    )
    .unwrap();
    let mut session = session();
    let name = path.to_str().unwrap();
    assert_eq!(
        session
            .invoke("file", &["mtime", name, "2200000000"])
            .unwrap(),
        "2200000000"
    );
    let stat = fs::metadata(&path).unwrap();
    assert_eq!(stat.atime(), 1_600_000_000);
    assert_eq!(stat.mtime(), 2_200_000_000);
    assert_eq!(stat.atime_nsec(), 0);
    assert_eq!(stat.mtime_nsec(), 0);
    assert_eq!(
        session.invoke("file", &["atime", name, "0x10"]).unwrap(),
        "16"
    );
    let stat = fs::metadata(&path).unwrap();
    assert_eq!(stat.atime(), 16);
    assert_eq!(stat.mtime(), 2_200_000_000);
    assert_eq!(
        session.invoke("file", &["mtime", name]).unwrap(),
        "2200000000"
    );
}
#[test]
fn setters_follow_links_and_work_for_directories() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("directory");
    fs::create_dir(&target).unwrap();
    let link = root.path().join("link");
    symlink(&target, &link).unwrap();
    let link_before = fs::symlink_metadata(&link).unwrap().mtime();
    let mut session = session();
    assert_eq!(
        session
            .invoke("file", &["mtime", link.to_str().unwrap(), "100"])
            .unwrap(),
        "100"
    );
    assert_eq!(fs::metadata(&target).unwrap().mtime(), 100);
    assert_eq!(fs::symlink_metadata(&link).unwrap().mtime(), link_before);
    assert_eq!(
        session
            .invoke("file", &["atime", target.to_str().unwrap(), "200"])
            .unwrap(),
        "200"
    );
    assert_eq!(fs::metadata(&target).unwrap().atime(), 200);
}
#[test]
fn lookup_and_integer_errors_are_catchable_and_do_not_create_or_modify_files() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    fs::write(&path, "payload").unwrap();
    let missing = root.path().join("missing");
    let mut session = session();
    session
        .invoke("set", &["path", missing.to_str().unwrap()])
        .unwrap();
    assert_eq!(
        session
            .invoke("catch", &["file mtime $path invalid", "message"])
            .unwrap(),
        "1"
    );
    assert_eq!(
        session.scalar("errorCode").unwrap(),
        "POSIX ENOENT {no such file or directory}"
    );
    assert!(!missing.exists());
    session
        .invoke("set", &["path", path.to_str().unwrap()])
        .unwrap();
    let before = fs::metadata(&path).unwrap();
    for value in ["invalid", "1.5", "9223372036854775808"] {
        assert!(
            session
                .invoke("file", &["mtime", path.to_str().unwrap(), value])
                .is_err()
        );
    }
    assert_eq!(fs::metadata(&path).unwrap().mtime(), before.mtime());
    assert_eq!(
        fs::metadata(&path).unwrap().mtime_nsec(),
        before.mtime_nsec()
    );
    session
        .invoke("file", &["mtime", path.to_str().unwrap(), "123"])
        .unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "payload");
}
#[test]
fn metadata_setters_cannot_mutate_existing_files_even_when_caught() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    fs::write(&path, "payload").unwrap();
    for field in ["atime", "mtime"] {
        let before = fs::metadata(&path).unwrap();
        let source = format!("name app; version 1; catch {{file {field} $path 0}}");
        let result = Session::open(
            &source,
            &BTreeMap::from([("path".into(), path.to_str().unwrap().into())]),
            &PortSpec::named("app"),
            PortGroups::default(),
        );
        assert!(result.is_err());
        let after = fs::metadata(&path).unwrap();
        assert_eq!(before.atime(), after.atime());
        assert_eq!(before.mtime(), after.mtime());
    }
}
