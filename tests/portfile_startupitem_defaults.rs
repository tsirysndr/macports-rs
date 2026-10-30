use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{collections::BTreeMap, fs};
fn session() -> Session {
    Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap()
}
fn enable(s: &mut Session) {
    s.enable_system(&BTreeMap::new(), |_| panic!("defaults need no subprocess"))
        .unwrap();
    s.initialize_creation_mask(0o022).unwrap();
}
#[test]
fn type_helper_is_read_only_and_requires_configuration_only_when_enabled() {
    let mut s = session();
    assert_eq!(
        s.invoke("portstartupitem::get_startupitem_type", &[])
            .unwrap(),
        "none"
    );
    s.invoke("startupitem.create", &["yes"]).unwrap();
    assert!(
        s.invoke("portstartupitem::get_startupitem_type", &[])
            .is_err()
    );
    s.invoke("set", &["system_options(startupitem_type)", "default"])
        .unwrap();
    for (platform, expected) in [("darwin", "launchd"), ("freebsd", "none")] {
        s.invoke("set", &["os.platform", platform]).unwrap();
        assert_eq!(
            s.invoke("portstartupitem::get_startupitem_type", &[])
                .unwrap(),
            expected
        );
    }
    s.invoke("set", &["system_options(startupitem_type)", "none"])
        .unwrap();
    assert_eq!(
        s.invoke("portstartupitem::get_startupitem_type", &[])
            .unwrap(),
        "none"
    );
    assert!(
        s.invoke("portstartupitem::get_startupitem_type", &["extra"])
            .is_err()
    );
}
#[test]
fn execution_defaults_follow_name_logfile_and_host_configuration_lazily() {
    let mut s = session();
    enable(&mut s);
    assert_eq!(s.scalar("startupitem.name").unwrap(), "app");
    assert_eq!(
        s.scalar("startupitem.plist").unwrap(),
        "org.macports.app.plist"
    );
    assert_eq!(s.scalar("startupitem.type").unwrap(), "none");
    assert_eq!(s.scalar("startupitem.autostart").unwrap(), "no");
    assert_eq!(s.scalar("startupitem.requires").unwrap(), "");
    s.invoke("option", &["startupitem.name", "worker one"])
        .unwrap();
    assert_eq!(
        s.scalar("startupitem.plist").unwrap(),
        "org.macports.worker one.plist"
    );
    s.invoke("option", &["startupitem.logfile", "/tmp/log with spaces"])
        .unwrap();
    assert_eq!(
        s.scalar("startupitem.logfile.stderr").unwrap(),
        "/tmp/log with spaces"
    );
    assert!(s.scalar("startupitem.install").is_err());
    s.invoke("set", &["system_options(startupitem_install)", "yes"])
        .unwrap();
    assert_eq!(s.scalar("startupitem.install").unwrap(), "yes");
    s.invoke("set", &["system_options(startupitem_type)", "default"])
        .unwrap();
    s.invoke("set", &["os.platform", "darwin"]).unwrap();
    s.invoke("startupitem.create", &["yes"]).unwrap();
    assert_eq!(s.scalar("startupitem.type").unwrap(), "launchd");
}
#[test]
fn explicit_portfile_defaults_and_overrides_survive_execution_initialization() {
    let mut s = Session::open("name app; version 1; startupitem.name worker; startupitem.plist custom.plist; startupitem.type none; startupitem.autostart yes", &BTreeMap::new(), &PortSpec::named("app"), PortGroups::default()).unwrap();
    enable(&mut s);
    assert_eq!(s.scalar("startupitem.name").unwrap(), "worker");
    assert_eq!(s.scalar("startupitem.plist").unwrap(), "custom.plist");
    assert_eq!(s.scalar("startupitem.type").unwrap(), "none");
    assert_eq!(s.scalar("startupitem.autostart").unwrap(), "yes");
}
#[test]
fn scalar_option_accessor_preserves_paths_and_names_with_spaces_during_staging() {
    let temp = tempfile::tempdir().unwrap();
    let stage = temp.path().join("stage");
    fs::create_dir(&stage).unwrap();
    let source = temp.path().join("custom {service} plist");
    fs::write(&source, b"payload").unwrap();
    let mut s = session();
    enable(&mut s);
    for (key, value) in [
        ("destroot", stage.to_str().unwrap()),
        ("prefix", "/opt/local"),
        ("startupitem.type", "launchd"),
        ("startupitem.name", "worker one"),
        ("startupitem.custom_file", source.to_str().unwrap()),
    ] {
        s.invoke("option", &[key, value]).unwrap();
    }
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    assert_eq!(
        fs::read(stage.join(
            "opt/local/etc/LaunchDaemons/org.macports.worker one/org.macports.worker one.plist"
        ))
        .unwrap(),
        b"payload"
    );
}
