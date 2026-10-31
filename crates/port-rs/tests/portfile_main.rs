use port_rs::{
    portfile::{PortGroups, Session, TargetOutcome},
    spec::PortSpec,
};
use std::collections::BTreeMap;
fn session(source: &str) -> Session {
    Session::open(
        source,
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap()
}
#[test]
fn main_metadata_provides_the_stateless_root_for_install_targets() {
    let mut s = session("name app; version 1");
    let handle = s.scalar("org.macports.main").unwrap();
    let item = &s.metadata().dependency_items[&handle];
    assert_eq!(item["name"], "org.macports.main");
    assert_eq!(item["procedure"], "portmain::main");
    assert_eq!(item["provides"], "main");
    assert_eq!(item["state"], "no");
    for field in ["requires", "runpkg", "prerun", "postrun"] {
        assert!(!item.contains_key(field));
    }
    assert_eq!(
        s.invoke("portmain::main", &["ignored", "args"]).unwrap(),
        "0"
    );
    s.enable_system(&BTreeMap::new(), |_| panic!("main has no process"))
        .unwrap();
    for _ in 0..2 {
        assert_eq!(
            s.execute_target(&handle, None, false).unwrap(),
            TargetOutcome::Completed
        );
    }
}
#[test]
fn main_hooks_and_override_use_normal_target_dispatch() {
    let mut s = session(
        r#"
        name app; version 1; set events {}
        pre-main {lappend events pre}
        main {lappend events body}
        post-main {lappend events post}
    "#,
    );
    let handle = s.scalar("org.macports.main").unwrap();
    assert_eq!(s.scalar("events").unwrap(), "");
    s.enable_system(&BTreeMap::new(), |_| panic!("main has no process"))
        .unwrap();
    assert_eq!(
        s.execute_target(&handle, None, true).unwrap(),
        TargetOutcome::DryRun
    );
    assert_eq!(s.scalar("events").unwrap(), "");
    s.execute_target(&handle, None, false).unwrap();
    assert_eq!(s.scalar("events").unwrap(), "pre body post");
}
