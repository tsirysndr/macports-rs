use port_rs::build::state::{Compatibility, LockStatus, StateFile};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    sync::mpsc,
    time::{Duration, UNIX_EPOCH},
};
#[test]
fn records_match_upstream_format_and_append_without_duplicate_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".macports.app.state");
    let mut state = StateFile::open(&path, Some(0o022)).unwrap();
    state.initialize("abcdef").unwrap();
    assert!(state.record("target", "org.macports.fetch").unwrap());
    assert!(!state.record("target", "org.macports.fetch").unwrap());
    assert!(state.contains("target", "org.macports.fetch").unwrap());
    assert!(!state.contains("target", "fetch").unwrap());
    assert_eq!(state.first("checksum").unwrap().as_deref(), Some("abcdef"));
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "version: 2\nchecksum: abcdef\ntarget: org.macports.fetch\n"
    );
}
#[test]
fn exclusivity_waiting_and_drop_release_the_same_file_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state");
    let mut first = StateFile::open(&path, None).unwrap();
    assert_eq!(first.lock_status, LockStatus::Exclusive);
    first.record("target", "first").unwrap();
    let (ready, receiver) = mpsc::channel();
    let (finished, done) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        assert!(StateFile::try_open(&path, None).unwrap().is_none());
        ready.send(()).unwrap();
        let mut state = StateFile::open(&path, None).unwrap();
        assert!(state.contains("target", "first").unwrap());
        state.record("target", "second").unwrap();
        finished.send(()).unwrap();
    });
    receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(done.try_recv().is_err());
    drop(first);
    done.recv_timeout(Duration::from_secs(5)).unwrap();
    worker.join().unwrap();
    let mut state = StateFile::try_open(&dir.path().join("state"), None)
        .unwrap()
        .unwrap();
    assert!(state.contains("target", "second").unwrap());
}
#[test]
fn checks_legacy_timestamps_current_checksums_and_unknown_versions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state");
    fs::write(&path, "target: org.macports.build\n").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(100)))
        .unwrap();
    let mut state = StateFile::open(&path, None).unwrap();
    assert_eq!(
        state.compatibility("any", 100).unwrap(),
        Compatibility::Current
    );
    assert_eq!(
        state.compatibility("any", 101).unwrap(),
        Compatibility::PortfileChanged
    );
    drop(state);
    for (contents, expected) in [
        ("version: 2\nchecksum: correct\n", Compatibility::Current),
        (
            "version: 2\nchecksum: old\n",
            Compatibility::PortfileChanged,
        ),
        ("version: 2\n", Compatibility::PortfileChanged),
        (
            "version: 9\n",
            Compatibility::UnsupportedVersion("9".into()),
        ),
    ] {
        fs::write(&path, contents).unwrap();
        let mut state = StateFile::open(&path, None).unwrap();
        assert_eq!(state.compatibility("correct", 0).unwrap(), expected);
    }
}
#[test]
fn compares_variants_with_empty_state_and_completed_target_rules() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state");
    let requested = BTreeMap::from([("ssl".into(), "+".into()), ("universal".into(), "-".into())]);
    let mut state = StateFile::open(&path, None).unwrap();
    state.initialize("checksum").unwrap();
    assert!(state.variants_match(&requested).unwrap().0);
    state.record("target", "org.macports.fetch").unwrap();
    assert!(!state.variants_match(&requested).unwrap().0);
    state.record("variant", "+ssl").unwrap();
    state.record("variant", "-universal").unwrap();
    assert_eq!(
        state.variants_match(&requested).unwrap(),
        (true, requested.clone())
    );
    state.record("variant", "-ssl").unwrap();
    let (matches, old) = state.variants_match(&requested).unwrap();
    assert!(!matches);
    assert_eq!(old["ssl"], "-");
}
#[test]
fn new_state_permissions_follow_mask_and_existing_permissions_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state");
    drop(StateFile::open(&path, Some(0o027)).unwrap());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    drop(StateFile::open(&path, Some(0)).unwrap());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
