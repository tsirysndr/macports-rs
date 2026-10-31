use port_rs::{
    portfile::{PortGroups, Session},
    progress::Progress,
    spec::PortSpec,
};
use std::{collections::BTreeMap, fs};
fn session(path: &std::path::Path) -> Session {
    let mut s = Session::open(
        r#"
        name app; version 1; set events {}
        proc portfetch::start_pings {} {global events; lappend events ping}
        proc portfetch::check_dns {} {global events; lappend events dns}
        set portfetch::fetch_urls {master_sites source.tar.gz}
        set all_dist_files source.tar.gz
    "#,
        &BTreeMap::from([("distpath".into(), path.to_str().unwrap().into())]),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    s.enable_system(&BTreeMap::new(), |_| panic!("no shell expected"))
        .unwrap();
    s.set_progress(Progress::new(true));
    s
}
#[test]
fn startup_creates_directory_but_cached_files_skip_preparation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("distfiles/app");
    let mut s = session(&path);
    s.invoke("portfetch::fetch_start", &["ignored"]).unwrap();
    assert!(path.is_dir());
    assert_eq!(s.scalar("events").unwrap(), "ping dns");
    fs::write(path.join("source.tar.gz"), "cached").unwrap();
    s.invoke("set", &["events", ""]).unwrap();
    s.invoke("set", &["portarchivefetch::attempted", "yes"])
        .unwrap();
    assert_eq!(s.invoke("portfetch::fetch_start", &[]).unwrap(), "0");
    assert_eq!(s.scalar("events").unwrap(), "");
    s.invoke("unset", &["all_dist_files"]).unwrap();
    s.invoke("portfetch::_fetch_start", &[]).unwrap();
    assert_eq!(s.scalar("events").unwrap(), "dns");
}
#[test]
fn ping_failure_prevents_directory_creation_and_privilege_failure_stops_dns() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("missing");
    let mut s = session(&path);
    s.invoke("proc", &["portfetch::start_pings", "", "error ping-failed"])
        .unwrap();
    assert!(
        s.invoke("portfetch::fetch_start", &[])
            .unwrap_err()
            .to_string()
            .contains("ping-failed")
    );
    assert!(!path.exists());
    let blocker = root.path().join("file");
    fs::write(&blocker, "regular file").unwrap();
    let mut s = session(&blocker.join("child"));
    s.invoke(
        "proc",
        &[
            "elevateToRoot",
            "phase",
            "global events; lappend events $phase; error cannot-elevate",
        ],
    )
    .unwrap();
    assert!(
        s.invoke("portfetch::_fetch_start", &[])
            .unwrap_err()
            .to_string()
            .contains("cannot-elevate")
    );
    assert_eq!(s.scalar("events").unwrap(), "ping fetch");
}
