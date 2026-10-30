use port_rs::{
    build::state::StateFile,
    portfile::{PortGroups, Session, TargetOutcome},
    spec::PortSpec,
};
use std::collections::BTreeMap;
fn session(execute: bool) -> Session {
    let mut s = Session::open(r#"
        name app
        version 1
        set events {}
        proc init name {global events; lappend events init}
        proc start name {global events; lappend events start}
        proc before name {global events; lappend events pre}
        proc body name {global events fail; lappend events body; if {[info exists fail] && $fail} {error failed}}
        proc after name {global events; lappend events post}
        proc finish name {global events; lappend events finish}
        set item [target_new org.macports.custom body]
        target_init $item init
        target_prerun $item start
        ditem_key $item pre {before}
        ditem_key $item post {after}
        target_postrun $item finish
    "#, &BTreeMap::new(), &PortSpec::named("app"), PortGroups::default()).unwrap();
    if execute {
        s.enable_system(&BTreeMap::new(), |_| {
            panic!("callbacks do not need a shell")
        })
        .unwrap();
    }
    s
}
#[test]
fn executes_callbacks_in_order_and_skips_completed_body_after_init() {
    let root = tempfile::tempdir().unwrap();
    let mut s = session(true);
    let item = s.scalar("item").unwrap();
    let fd = s
        .attach_build_state(StateFile::open(&root.path().join("state"), None).unwrap())
        .unwrap();
    assert_eq!(
        s.execute_target(&item, Some(&fd), false).unwrap(),
        TargetOutcome::Completed
    );
    assert_eq!(
        s.scalar("events").unwrap(),
        "init start pre body post finish"
    );
    s.invoke("set", &["events", ""]).unwrap();
    assert_eq!(
        s.execute_target(&item, Some(&fd), false).unwrap(),
        TargetOutcome::AlreadyCompleted
    );
    assert_eq!(s.scalar("events").unwrap(), "init");
}
#[test]
fn failures_stop_later_callbacks_do_not_mark_complete_and_can_retry() {
    let root = tempfile::tempdir().unwrap();
    let mut s = session(true);
    let item = s.scalar("item").unwrap();
    let fd = s
        .attach_build_state(StateFile::open(&root.path().join("state"), None).unwrap())
        .unwrap();
    s.invoke("set", &["fail", "1"]).unwrap();
    assert!(
        s.execute_target(&item, Some(&fd), false)
            .unwrap_err()
            .to_string()
            .contains("failed")
    );
    assert_eq!(s.scalar("events").unwrap(), "init start pre body");
    assert_eq!(
        s.invoke("check_statefile", &["target", "org.macports.custom", &fd])
            .unwrap(),
        "0"
    );
    s.invoke("set", &["fail", "0"]).unwrap();
    assert_eq!(
        s.execute_target(&item, Some(&fd), false).unwrap(),
        TargetOutcome::Completed
    );
}
#[test]
fn dry_run_only_initializes_and_always_targets_are_not_recorded() {
    let root = tempfile::tempdir().unwrap();
    let mut s = session(true);
    let item = s.scalar("item").unwrap();
    let fd = s
        .attach_build_state(StateFile::open(&root.path().join("state"), None).unwrap())
        .unwrap();
    assert_eq!(
        s.execute_target(&item, Some(&fd), true).unwrap(),
        TargetOutcome::DryRun
    );
    assert_eq!(s.scalar("events").unwrap(), "init");
    assert_eq!(
        s.invoke("check_statefile", &["target", "org.macports.custom", &fd])
            .unwrap(),
        "0"
    );
    s.invoke("target_runtype", &[&item, "always"]).unwrap();
    for _ in 0..2 {
        assert_eq!(
            s.execute_target(&item, Some(&fd), false).unwrap(),
            TargetOutcome::Completed
        );
    }
    assert_eq!(
        s.invoke("check_statefile", &["target", "org.macports.custom", &fd])
            .unwrap(),
        "0"
    );
}
#[test]
fn stateless_target_needs_no_handle_and_invalid_context_cannot_run_init() {
    let mut s = session(false);
    let item = s.scalar("item").unwrap();
    assert!(s.execute_target(&item, None, false).is_err());
    assert_eq!(s.scalar("events").unwrap(), "");
    let mut s = session(true);
    let item = s.scalar("item").unwrap();
    assert!(s.execute_target(&item, None, false).is_err());
    assert!(s.execute_target(&item, Some("missing"), false).is_err());
    assert_eq!(s.scalar("events").unwrap(), "");
    s.invoke("target_state", &[&item, "no"]).unwrap();
    assert_eq!(
        s.execute_target(&item, None, false).unwrap(),
        TargetOutcome::Completed
    );
}
#[test]
fn postrun_failure_is_retryable_and_caught_unsupported_commands_never_complete() {
    let root = tempfile::tempdir().unwrap();
    let mut s = session(true);
    let item = s.scalar("item").unwrap();
    let fd = s
        .attach_build_state(StateFile::open(&root.path().join("state"), None).unwrap())
        .unwrap();
    s.invoke("proc", &["finish", "name", "error finish-failed"])
        .unwrap();
    assert!(s.execute_target(&item, Some(&fd), false).is_err());
    assert_eq!(
        s.invoke("check_statefile", &["target", "org.macports.custom", &fd])
            .unwrap(),
        "0"
    );
    s.invoke("proc", &["finish", "name", "catch {exec must-not-run}"])
        .unwrap();
    assert!(s.execute_target(&item, Some(&fd), false).is_err());
    drop(s);
    let mut state = StateFile::open(&root.path().join("state"), None).unwrap();
    assert!(!state.contains("target", "org.macports.custom").unwrap());
}

#[test]
fn callbacks_can_change_later_hooks_and_completion_policy() {
    let root = tempfile::tempdir().unwrap();
    let mut s = session(true);
    let item = s.scalar("item").unwrap();
    let fd = s
        .attach_build_state(StateFile::open(&root.path().join("state"), None).unwrap())
        .unwrap();
    s.invoke("proc", &["body", "name", "global item events; lappend events body; ditem_key $item post {after after}; target_runtype $item always"]).unwrap();
    assert_eq!(
        s.execute_target(&item, Some(&fd), false).unwrap(),
        TargetOutcome::Completed
    );
    assert_eq!(
        s.scalar("events").unwrap(),
        "init start pre body post post finish"
    );
    assert_eq!(
        s.invoke("check_statefile", &["target", "org.macports.custom", &fd])
            .unwrap(),
        "0"
    );
}

#[test]
fn registered_portfile_override_and_hooks_execute_through_target_metadata() {
    let mut s = Session::open(
        "name app; version 1; set events {}; pre-build {lappend events before}; build {lappend events build}; post-build {lappend events after}",
        &BTreeMap::new(), &PortSpec::named("app"), PortGroups::default()).unwrap();
    s.enable_system(&BTreeMap::new(), |_| panic!("no shell required"))
        .unwrap();
    let item = s.scalar("org.macports.build").unwrap();
    // This fixture prepares the runtime callbacks itself. Built-in phase
    // initialization and package loading remain the host driver's job.
    s.invoke("ditem_key", &[&item, "prerun", ""]).unwrap();
    s.invoke("target_state", &[&item, "no"]).unwrap();
    assert_eq!(
        s.execute_target(&item, None, false).unwrap(),
        TargetOutcome::Completed
    );
    assert_eq!(s.scalar("events").unwrap(), "before build after");
}
