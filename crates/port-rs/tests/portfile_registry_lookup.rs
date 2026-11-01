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

#[test]
fn startup_binding_is_available_to_groups_and_sees_a_later_created_registry() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("later.db");
    let mut groups = PortGroups::default();
    groups
        .insert(
            "probe",
            "1.0",
            "set group_found [registry_exists_for_name app]".into(),
        )
        .unwrap();
    let mut s = Session::open_with_registry(
        "name app; version 1; PortGroup probe 1.0; set found [registry_exists app 1]",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        groups,
        &BTreeMap::new(),
        None,
        Some(&path),
    )
    .unwrap();
    assert_eq!(s.scalar("group_found").unwrap(), "0");
    assert_eq!(s.scalar("found").unwrap(), "0");
    assert!(!path.exists());
    let mut writer = Writer::create(&path).unwrap();
    let mut tx = writer.transaction().unwrap();
    tx.register(&NewImage {
        name: "app".into(),
        version: "1".into(),
        location: "/tmp/archive".into(),
        archs: "arm64".into(),
        ..Default::default()
    })
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(s.invoke("registry_exists", &["app", "1"]).unwrap(), "1");
}

#[test]
fn dependency_ports_and_active_entries_follow_upstream() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("registry.db");
    let mut writer = Writer::create(&path).unwrap();
    let mut tx = writer.transaction().unwrap();
    let id = tx
        .register(&NewImage {
            name: "zstd".into(),
            version: "1.5".into(),
            revision: 2,
            variants: "+universal".into(),
            location: "/tmp/zstd".into(),
            archs: "arm64".into(),
            ..Default::default()
        })
        .unwrap();
    tx.activate(id, &[]).unwrap();
    tx.commit().unwrap();
    let mut s = session();
    s.set_registry_lookup(Some(&path));
    assert_eq!(s.invoke("_get_dep_port", &["port:zstd"]).unwrap(), "zstd");
    s.invoke("set", &["prefix", tmp.path().to_str().unwrap()])
        .unwrap();
    s.invoke("set", &["frameworks_dir", "/nonexistent"])
        .unwrap();
    s.invoke("set", &["os.platform", "darwin"]).unwrap();
    assert_eq!(
        s.invoke("_get_dep_port", &["path:lib/libnothing.dylib:nothing"])
            .unwrap(),
        "nothing"
    );
    assert_eq!(
        s.invoke("registry_active", &["ZSTD"]).unwrap(),
        "{zstd 1.5 2 +universal 1 0}"
    );
    assert_eq!(
        s.invoke("registry_active", &["missing"])
            .unwrap_err()
            .to_string(),
        "Portfile evaluation: Registry error: missing not registered as installed & active."
    );
}

#[test]
fn hooks_deactivate_replaced_ports_as_upstream() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("registry.db");
    let mut writer = Writer::create(&path).unwrap();
    let mut tx = writer.transaction().unwrap();
    let id = tx
        .register(&NewImage {
            name: "old".into(),
            version: "1".into(),
            location: "/tmp/old".into(),
            archs: "arm64".into(),
            ..Default::default()
        })
        .unwrap();
    tx.activate(id, &[]).unwrap();
    tx.commit().unwrap();
    drop(writer);
    let mut s = session();
    s.set_registry_lookup(Some(&path));
    assert!(
        s.invoke("registry_deactivate_composite", &["old", "2"])
            .unwrap_err()
            .to_string()
            .ends_with("Active version of old is not 2 but 1_0.")
    );
    s.invoke(
        "registry_deactivate_composite",
        &["old", "", "ports_nodepcheck 1"],
    )
    .unwrap();
    assert!(s.invoke("registry_active", &["old"]).is_err());
    assert_eq!(
        s.invoke("registry_deactivate_composite", &["old"]).unwrap(),
        ""
    );
    assert!(
        s.invoke("registry_deactivate_composite", &["missing"])
            .unwrap_err()
            .to_string()
            .ends_with("Image error: port missing is not active.")
    );
}
