use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{collections::BTreeMap, fs, os::unix::fs::symlink, path::Path};
fn session(root: &Path, execute: bool, dry: bool) -> Session {
    fs::write(root.join("Portfile"), "name app; version 1").unwrap();
    let vars = BTreeMap::from([
        ("portpath".into(), root.to_str().unwrap().into()),
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
            "worksymlink".into(),
            root.join("work").to_str().unwrap().into(),
        ),
        ("place_worksymlink".into(), "yes".into()),
        ("ports_dryrun".into(), dry.to_string()),
    ]);
    let mut session = Session::open(
        "name app; version 1",
        &vars,
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    if execute {
        session
            .enable_system(&BTreeMap::new(), |_| panic!("state opening is native"))
            .unwrap();
    }
    session
}
#[test]
fn native_open_creates_work_and_exposes_locked_records_in_procedures() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let mut s = session(&root, true, false);
    s.invoke(
        "proc",
        &["open_build", "", "open_statefile ignored arguments"],
    )
    .unwrap();
    let fd = s.invoke("open_build", &[]).unwrap();
    let work = root.join("build/new/app/work");
    assert!(work.join(".home").is_dir());
    assert!(work.join(".tmp").is_dir());
    assert_eq!(fs::read_link(root.join("work")).unwrap(), work);
    assert_eq!(
        s.invoke("check_statefile", &["version", "2", &fd]).unwrap(),
        "1"
    );
    s.invoke("write_statefile", &["target", "org.macports.build", &fd])
        .unwrap();
    s.invoke("close", &[&fd]).unwrap();
    let fd = s.invoke("open_statefile", &[]).unwrap();
    assert_eq!(
        s.invoke("check_statefile", &["target", "org.macports.build", &fd])
            .unwrap(),
        "1"
    );
}
#[test]
fn missing_dry_run_state_creates_no_work_or_link() {
    let tmp = tempfile::tempdir().unwrap();
    let mut s = session(tmp.path(), true, true);
    let fd = s.invoke("open_statefile", &[]).unwrap();
    assert_eq!(
        s.invoke("check_statefile", &["version", "2", &fd]).unwrap(),
        "0"
    );
    s.invoke("write_statefile", &["target", "org.macports.build", &fd])
        .unwrap();
    s.invoke("close", &[&fd]).unwrap();
    assert!(!tmp.path().join("build").exists());
    assert!(!tmp.path().join("work").exists());
}
#[test]
fn dry_run_keeps_stale_state_and_real_open_cleans_it() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let mut s = session(&root, true, false);
    let fd = s.invoke("open_statefile", &[]).unwrap();
    s.invoke("close", &[&fd]).unwrap();
    let work = root.join("build/new/app/work");
    let path = work.join(".macports.app.state");
    let before = fs::read(&path).unwrap();
    fs::write(work.join("artifact"), "old").unwrap();
    fs::write(root.join("Portfile"), "name app; version 2").unwrap();
    s.invoke("set", &["ports_dryrun", "yes"]).unwrap();
    let fd = s.invoke("open_statefile", &[]).unwrap();
    s.invoke("close", &[&fd]).unwrap();
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(work.join("artifact").exists());
    s.invoke("set", &["ports_dryrun", "no"]).unwrap();
    let fd = s.invoke("open_statefile", &[]).unwrap();
    assert!(!work.join("artifact").exists());
    assert_ne!(fs::read(&path).unwrap(), before);
    s.invoke("close", &[&fd]).unwrap();
}
#[test]
fn repairs_dangling_work_link_and_metadata_open_cannot_mutate() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let mut denied = session(&root, false, false);
    assert!(denied.invoke("open_statefile", &[]).is_err());
    assert!(!root.join("build").exists());
    let mut s = session(&root, true, false);
    symlink(root.join("missing"), root.join("work")).unwrap();
    let fd = s.invoke("open_statefile", &[]).unwrap();
    assert_eq!(
        fs::read_link(root.join("work")).unwrap(),
        root.join("build/new/app/work")
    );
    s.invoke("close", &[&fd]).unwrap();
}
