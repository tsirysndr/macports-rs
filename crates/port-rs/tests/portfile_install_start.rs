use port_rs::{
    portfile::{PortGroups, Session},
    progress::Progress,
    spec::PortSpec,
};
use std::collections::BTreeMap;

fn session(source: &str, prefix: &str) -> Session {
    let mut session = Session::open(
        source,
        &BTreeMap::from([("prefix_frozen".into(), prefix.into())]),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    session.set_progress(Progress::new(true));
    session
        .enable_system(&BTreeMap::new(), |_| panic!("no subprocess expected"))
        .unwrap();
    session
}
#[test]
fn startup_dispatches_user_options_and_groups_as_literal_words() {
    let root = tempfile::tempdir().unwrap();
    let mut s = session(
        r#"
        name app; version 1
        set events {}
        proc addgroup group {global events; lappend events [list group $group]}
        proc existsgroup group {return 42}
        proc adduser {name args} {global events; lappend events [list user $name $args]}
        add_users app group=staff {realname=App Worker} {home=/tmp/[unsafe]} lone
        add_users-append app shell=/bin/false
    "#,
        root.path().to_str().unwrap(),
    );
    s.invoke("portinstall::install_start", &["ignored", "args"])
        .unwrap();
    assert_eq!(
        s.scalar("events").unwrap(),
        "{group staff} {user app {gid=42 {realname=App Worker} {home=/tmp/[unsafe]} shell=/bin/false}}"
    );
}
#[test]
fn account_failure_stops_before_user_creation() {
    let root = tempfile::tempdir().unwrap();
    let mut s = session(
        r#"
        name app; version 1
        set called no
        proc addgroup group {error group-failed}
        proc adduser args {global called; set called yes}
        add_users app group=staff
    "#,
        root.path().to_str().unwrap(),
    );
    assert!(
        s.invoke("portinstall::install_start", &[])
            .unwrap_err()
            .to_string()
            .contains("group-failed")
    );
    assert_eq!(s.scalar("called").unwrap(), "no");
}
#[test]
fn unwritable_prefix_stops_before_account_dispatch() {
    let root = tempfile::tempdir().unwrap();
    let mut s = session(
        r#"
        name app; version 1
        set called no
        proc handle_add_users {} {global called; set called yes}
    "#,
        root.path().join("missing").to_str().unwrap(),
    );
    assert!(
        s.invoke("portinstall::install_start", &[])
            .unwrap_err()
            .to_string()
            .contains("install privilege lifecycle")
    );
    // Unsupported privilege handling poisons the session, preventing later work.
    assert!(s.invoke("handle_add_users", &[]).is_err());
}
