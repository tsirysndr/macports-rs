use port_rs::{
    build::state::StateFile,
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{collections::BTreeMap, fs};
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
                panic!("state helpers need no subprocess")
            })
            .unwrap();
    }
    session
}
#[test]
fn records_are_visible_to_native_helpers_and_close_releases_lock() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state");
    let file = StateFile::open(&path, None).unwrap();
    let mut session = session(true);
    let fd = session.attach_build_state(file).unwrap();
    assert!(StateFile::try_open(&path, None).unwrap().is_none());
    assert_eq!(
        session
            .invoke("check_statefile", &["target", "org.macports.build", &fd])
            .unwrap(),
        "0"
    );
    assert_eq!(
        session
            .invoke("write_statefile", &["target", "org.macports.build", &fd])
            .unwrap(),
        ""
    );
    assert_eq!(
        session
            .invoke("write_statefile", &["target", "org.macports.build", &fd])
            .unwrap(),
        "0"
    );
    assert_eq!(
        session
            .invoke("check_statefile", &["target", "org.macports.build", &fd])
            .unwrap(),
        "1"
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "target: org.macports.build\n"
    );
    session.invoke("close", &[&fd]).unwrap();
    assert!(StateFile::try_open(&path, None).unwrap().is_some());
    assert!(
        session
            .invoke("check_statefile", &["target", "org.macports.build", &fd])
            .is_err()
    );
}
#[test]
fn state_values_update_callers_variables_and_leave_missing_results_untouched() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state");
    fs::write(&path, "version: 2\nchecksum: first\nchecksum: second\n").unwrap();
    let mut session = session(true);
    let fd = session
        .attach_build_state(StateFile::open(&path, None).unwrap())
        .unwrap();
    session.invoke("proc", &["read_state","fd","set result unchanged; set found [get_statefile_value checksum $fd result]; list $found $result"]).unwrap();
    assert_eq!(session.invoke("read_state", &[&fd]).unwrap(), "1 first");
    session.invoke("set", &["result", "unchanged"]).unwrap();
    assert_eq!(
        session
            .invoke("get_statefile_value", &["absent", &fd, "result"])
            .unwrap(),
        "0"
    );
    assert_eq!(session.scalar("result").unwrap(), "unchanged");
    assert_eq!(
        session
            .invoke("get_statefile_value", &["check.*", &fd, "array(value)"])
            .unwrap(),
        "1"
    );
    assert_eq!(session.invoke("set", &["array(value)"]).unwrap(), "first");
}
#[test]
fn variant_comparison_uses_caller_dicts_and_empty_state_skips_missing_input() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state");
    let mut session = session(true);
    let fd = session
        .attach_build_state(StateFile::open(&path, None).unwrap())
        .unwrap();
    assert_eq!(
        session
            .invoke("check_statefile_variants", &["not_defined", "old", &fd])
            .unwrap(),
        "0"
    );
    assert_eq!(session.scalar("old").unwrap(), "");
    session
        .invoke("write_statefile", &["target", "org.macports.fetch", &fd])
        .unwrap();
    session
        .invoke("write_statefile", &["variant", "+ssl", &fd])
        .unwrap();
    session.invoke("proc", &["compare","fd","set wanted {ssl +}; set matches [check_statefile_variants wanted old $fd]; list $matches $old"]).unwrap();
    assert_eq!(session.invoke("compare", &[&fd]).unwrap(), "0 {ssl +}");
    session.invoke("set", &["wanted", "ssl -"]).unwrap();
    assert_eq!(
        session
            .invoke("check_statefile_variants", &["wanted", "old", &fd])
            .unwrap(),
        "1"
    );
    assert_eq!(session.scalar("old").unwrap(), "ssl +");
}
#[test]
fn session_lifetime_and_host_close_release_handles_and_metadata_cannot_attach() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state");
    assert!(
        session(false)
            .attach_build_state(StateFile::open(&path, None).unwrap())
            .is_err()
    );
    let mut first = session(true);
    let fd = first
        .attach_build_state(StateFile::open(&path, None).unwrap())
        .unwrap();
    first.close_build_state(&fd).unwrap();
    assert!(first.close_build_state(&fd).is_err());
    let fd2 = first
        .attach_build_state(StateFile::open(&path, None).unwrap())
        .unwrap();
    assert_ne!(fd, fd2);
    drop(first);
    assert!(StateFile::try_open(&path, None).unwrap().is_some());
}

#[test]
fn host_open_initializes_and_attaches_state_only_for_execution_sessions() {
    let root = tempfile::tempdir().unwrap();
    let work = root.path().join("work");
    fs::create_dir(&work).unwrap();
    let source = root.path().join("Portfile");
    fs::write(&source, "name app; version 1").unwrap();
    assert!(
        session(false)
            .open_build_state(&work, "app", &source, false)
            .is_err()
    );
    assert_eq!(fs::read_dir(&work).unwrap().count(), 0);
    let mut session = session(true);
    let (fd, compatibility) = session
        .open_build_state(&work, "app", &source, false)
        .unwrap();
    assert_eq!(compatibility, port_rs::build::state::Compatibility::Current);
    assert_eq!(
        session
            .invoke("check_statefile", &["version", "2", &fd])
            .unwrap(),
        "1"
    );
    session.invoke("close", &[&fd]).unwrap();
    assert!(
        StateFile::try_open(&work.join(".macports.app.state"), None)
            .unwrap()
            .is_some()
    );
}
