use port_rs::{
    portfile::{PortGroups, Session},
    registry::write::{NewImage, Writer},
    spec::PortSpec,
};
use std::collections::BTreeMap;
fn session() -> Session {
    Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap()
}
#[test]
fn registry_aliases_use_fresh_readonly_snapshots_and_native_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("registry.db");
    let mut writer = Writer::create(&path).unwrap();
    let mut s = session();
    s.set_registry_lookup(Some(&path));
    assert_eq!(s.invoke("registry_exists_for_name", &["app"]).unwrap(), "0");
    let mut tx = writer.transaction().unwrap();
    tx.register(&NewImage {
        name: "app".into(),
        version: "1".into(),
        epoch: 7,
        location: "/tmp/image".into(),
        archs: "arm64".into(),
        ..Default::default()
    })
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(s.invoke("registry_exists", &["APP", "1"]).unwrap(), "1");
    assert_eq!(s.invoke("registry_exists_for_name", &["App"]).unwrap(), "1");
    for args in [
        vec!["app", "1", "1"],
        vec!["app", "1", "0", "+ssl"],
        vec!["app", "2"],
        vec!["app' OR 1=1--", "1"],
    ] {
        assert_eq!(s.invoke("registry_exists", &args).unwrap(), "0");
    }
    assert!(s.invoke("registry_exists", &["app"]).is_err());
    s.set_registry_lookup(None);
    assert_eq!(s.invoke("registry_exists", &["app", "1"]).unwrap(), "0");
    let missing = tmp.path().join("missing.db");
    s.set_registry_lookup(Some(&missing));
    assert_eq!(s.invoke("registry_exists", &["app", "1"]).unwrap(), "0");
    assert!(!missing.exists());
}
#[test]
fn unbound_registry_is_explicitly_unsupported() {
    assert!(session().invoke("registry_exists", &["app", "1"]).is_err());
}
