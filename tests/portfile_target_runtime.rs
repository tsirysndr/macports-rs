use port_rs::{
    build::{ProcessResult, state::StateFile},
    portfile::{PortGroups, Session, SystemRequest, TargetOutcome},
    spec::PortSpec,
};
use std::{
    cell::RefCell, collections::BTreeMap, fs, os::unix::process::ExitStatusExt, path::Path, rc::Rc,
};
fn session(root: &Path) -> (Session, Rc<RefCell<Vec<SystemRequest>>>) {
    let source = r#"
        name app
        version 1
        proc init name {system init}
        proc start name {system start}
        proc body name {global fail; system body; if {[info exists fail] && $fail} {error body-failed}}
        proc after name {system post}
        proc finish name {system finish}
        set item [target_new org.macports.custom body]
        target_init $item init
        target_prerun $item start
        ditem_key $item post {after}
        target_postrun $item finish
    "#;
    fs::write(root.join("Portfile"), source).unwrap();
    let vars = BTreeMap::from([
        ("portpath".into(), root.to_str().unwrap().into()),
        (
            "portdbpath".into(),
            root.join("database").to_str().unwrap().into(),
        ),
        (
            "portbuildpath".into(),
            root.join("build/new").to_str().unwrap().into(),
        ),
        (
            "subbuildpath".into(),
            root.join("build/new/app").to_str().unwrap().into(),
        ),
        (
            "workpath".into(),
            root.join("build/new/app/work").to_str().unwrap().into(),
        ),
        (
            "configure.developer_dir".into(),
            "/selected/Developer".into(),
        ),
    ]);
    let mut s = Session::open(
        source,
        &vars,
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    let requests = Rc::new(RefCell::new(Vec::new()));
    let output = requests.clone();
    s.enable_system(
        &BTreeMap::from([
            ("DEVELOPER_DIR".into(), "inherited".into()),
            ("HOME".into(), "original-home".into()),
            ("TMPDIR".into(), "original-tmp".into()),
        ]),
        move |request| {
            output.borrow_mut().push(request);
            Ok(ProcessResult {
                pid: 1,
                status: std::process::ExitStatus::from_raw(0),
            })
        },
    )
    .unwrap();
    (s, requests)
}
#[test]
fn runtime_scopes_environment_and_releases_state_after_success() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let (mut s, requests) = session(&root);
    let item = s.scalar("item").unwrap();
    assert_eq!(
        s.run_prepared_target(&item, false).unwrap(),
        TargetOutcome::Completed
    );
    let requests = requests.borrow();
    assert_eq!(
        requests
            .iter()
            .map(|r| r.script.as_str())
            .collect::<Vec<_>>(),
        ["init", "start", "body", "post", "finish"]
    );
    for r in requests.iter() {
        assert_eq!(
            r.environment["HOME"],
            root.join("build/new/app/work/.home").to_str().unwrap()
        );
        assert_eq!(
            r.environment["TMPDIR"],
            root.join("build/new/app/work/.tmp").to_str().unwrap()
        );
    }
    assert_eq!(requests[0].environment["DEVELOPER_DIR"], "inherited");
    assert_eq!(requests[1].environment["DEVELOPER_DIR"], "inherited");
    assert_eq!(
        requests[2].environment["DEVELOPER_DIR"],
        "/selected/Developer"
    );
    assert_eq!(
        requests[3].environment["DEVELOPER_DIR"],
        "/selected/Developer"
    );
    assert!(!requests[4].environment.contains_key("DEVELOPER_DIR"));
    assert_eq!(
        s.invoke("set", &["env(HOME)"]).unwrap(),
        root.join("database/home").to_str().unwrap()
    );
    for key in [
        "env(TMPDIR)",
        "env(DEVELOPER_DIR)",
        "target_state_fd",
        "ports_dryrun",
    ] {
        assert_eq!(s.invoke("info", &["exists", key]).unwrap(), "0");
    }
    assert!(
        StateFile::try_open(&root.join("build/new/app/work/.macports.app.state"), None)
            .unwrap()
            .is_some()
    );
}
#[test]
fn failed_phase_cleans_environment_unlocks_and_is_retryable() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut s, _) = session(tmp.path());
    let item = s.scalar("item").unwrap();
    s.invoke("set", &["fail", "1"]).unwrap();
    assert!(
        s.run_prepared_target(&item, false)
            .unwrap_err()
            .to_string()
            .contains("body-failed")
    );
    for key in ["env(TMPDIR)", "env(DEVELOPER_DIR)", "target_state_fd"] {
        assert_eq!(s.invoke("info", &["exists", key]).unwrap(), "0");
    }
    let mut state = StateFile::try_open(
        &tmp.path().join("build/new/app/work/.macports.app.state"),
        None,
    )
    .unwrap()
    .unwrap();
    assert!(!state.contains("target", "org.macports.custom").unwrap());
    drop(state);
    s.invoke("set", &["fail", "0"]).unwrap();
    assert_eq!(
        s.run_prepared_target(&item, false).unwrap(),
        TargetOutcome::Completed
    );
}
#[test]
fn dry_run_restores_flag_creates_no_work_and_preserves_developer_setting() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut s, requests) = session(tmp.path());
    let item = s.scalar("item").unwrap();
    s.invoke("set", &["ports_dryrun", "no"]).unwrap();
    assert_eq!(
        s.run_prepared_target(&item, true).unwrap(),
        TargetOutcome::DryRun
    );
    assert_eq!(requests.borrow().len(), 1);
    assert_eq!(s.scalar("ports_dryrun").unwrap(), "no");
    assert_eq!(
        s.invoke("set", &["env(DEVELOPER_DIR)"]).unwrap(),
        "inherited"
    );
    assert!(!tmp.path().join("build").exists());
}
#[test]
fn caught_unsupported_operation_still_releases_owned_lock() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut s, _) = session(tmp.path());
    let item = s.scalar("item").unwrap();
    s.invoke("proc", &["body", "name", "catch {exec must-not-run}"])
        .unwrap();
    assert!(s.run_prepared_target(&item, false).is_err());
    // Check while the poisoned session is still alive: Drop alone is too late.
    let mut state = StateFile::try_open(
        &tmp.path().join("build/new/app/work/.macports.app.state"),
        None,
    )
    .unwrap()
    .unwrap();
    assert!(!state.contains("target", "org.macports.custom").unwrap());
}

#[test]
fn state_open_failure_restores_environment_and_dry_run_option() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut s, requests) = session(tmp.path());
    let item = s.scalar("item").unwrap();
    fs::remove_file(tmp.path().join("Portfile")).unwrap();
    assert!(s.run_prepared_target(&item, false).is_err());
    assert!(requests.borrow().is_empty());
    assert_eq!(
        s.invoke("set", &["env(HOME)"]).unwrap(),
        tmp.path().join("database/home").to_str().unwrap()
    );
    assert_eq!(
        s.invoke("set", &["env(DEVELOPER_DIR)"]).unwrap(),
        "inherited"
    );
    for key in ["env(TMPDIR)", "target_state_fd", "ports_dryrun"] {
        assert_eq!(s.invoke("info", &["exists", key]).unwrap(), "0");
    }
}
