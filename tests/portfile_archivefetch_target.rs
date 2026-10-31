use port_rs::{
    build::state::StateFile,
    portfile::{PortGroups, Session, TargetOutcome},
    spec::PortSpec,
};
use std::collections::BTreeMap;

fn session(body: &str) -> Session {
    Session::open(
        &format!("name app; version 1; {body}"),
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap()
}

#[test]
fn archivefetch_metadata_satisfies_install_prerequisite_without_running_fetch() {
    let mut session = session("");
    let item = session.scalar("org.macports.archivefetch").unwrap();
    let fields = &session.metadata().dependency_items[&item];
    for (key, expected) in [
        ("name", "org.macports.archivefetch"),
        ("procedure", "portarchivefetch::archivefetch_main"),
        ("provides", "archivefetch"),
        ("requires", "main"),
        ("runtype", "always"),
        ("prerun", "portarchivefetch::archivefetch_start"),
        ("runpkg", "portarchivefetch_run"),
    ] {
        assert_eq!(fields[key], expected);
    }
    assert_ne!(fields.get("state").map(String::as_str), Some("no"));
    assert!(session.execute_target(&item, None, false).is_err());
}

#[test]
fn archivefetch_hooks_wrap_custom_body_and_do_not_mark_always_target_complete() {
    let mut session = session(
        r#"
        set events {}
        proc portarchivefetch::archivefetch_start args {global events; lappend events start}
        pre-archivefetch {lappend events pre}
        archivefetch {lappend events body}
        post-archivefetch {lappend events post}
    "#,
    );
    assert_eq!(
        session.metadata().targets["archivefetch"],
        "lappend events body"
    );
    assert_eq!(session.scalar("events").unwrap(), "");
    session
        .enable_system(&BTreeMap::new(), |_| panic!("no subprocess expected"))
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let handle = session
        .attach_build_state(StateFile::open(&root.path().join("state"), None).unwrap())
        .unwrap();
    let item = session.scalar("org.macports.archivefetch").unwrap();
    assert_eq!(
        session.execute_target(&item, Some(&handle), true).unwrap(),
        TargetOutcome::DryRun
    );
    assert_eq!(session.scalar("events").unwrap(), "");
    for _ in 0..2 {
        assert_eq!(
            session.execute_target(&item, Some(&handle), false).unwrap(),
            TargetOutcome::Completed
        );
    }
    assert_eq!(
        session.scalar("events").unwrap(),
        "start pre body post start pre body post"
    );
    assert_eq!(
        session
            .invoke(
                "check_statefile",
                &["target", "org.macports.archivefetch", &handle]
            )
            .unwrap(),
        "0"
    );
}

#[test]
fn failed_archivefetch_hook_stops_the_body_and_post_hook() {
    let mut session = session(
        r#"
        set events {}
        proc portarchivefetch::archivefetch_start args {global events; lappend events start}
        pre-archivefetch {error failed-before-fetch}
        archivefetch {lappend events body}
        post-archivefetch {lappend events post}
    "#,
    );
    session
        .enable_system(&BTreeMap::new(), |_| panic!("no subprocess expected"))
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let handle = session
        .attach_build_state(StateFile::open(&root.path().join("state"), None).unwrap())
        .unwrap();
    let item = session.scalar("org.macports.archivefetch").unwrap();
    assert!(
        session
            .execute_target(&item, Some(&handle), false)
            .unwrap_err()
            .to_string()
            .contains("failed-before-fetch")
    );
    assert_eq!(session.scalar("events").unwrap(), "start");
    assert_eq!(
        session
            .invoke(
                "check_statefile",
                &["target", "org.macports.archivefetch", &handle]
            )
            .unwrap(),
        "0"
    );
}
