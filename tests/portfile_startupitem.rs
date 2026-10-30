use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};
fn session(root: &Path) -> Session {
    fs::create_dir(root.join("stage")).unwrap();
    fs::write(root.join("input.plist"), b"custom plist\0\xff").unwrap();
    let vars = BTreeMap::from([
        (
            "destroot".into(),
            root.join("stage").to_str().unwrap().into(),
        ),
        ("prefix".into(), "/opt/local".into()),
        ("startupitem.type".into(), "launchd".into()),
        (
            "startupitem.custom_file".into(),
            root.join("input.plist").to_str().unwrap().into(),
        ),
    ]);
    let mut s = Session::open(
        "name app; version 1",
        &vars,
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    s.enable_system(&BTreeMap::new(), |_| {
        panic!("staging must not run external commands")
    })
    .unwrap();
    s.initialize_creation_mask(0o022).unwrap();
    s
}
#[test]
fn stages_custom_plist_preserving_bytes_mode_and_time_with_absolute_link() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = session(temp.path());
    let source = temp.path().join("input.plist");
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(123456789);
    fs::File::open(&source)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(time))
        .unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o440)).unwrap();
    s.invoke("startupitem.install", &["yes"]).unwrap();
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    let base = temp.path().join("stage/opt/local/etc/LaunchDaemons");
    let staged = base.join("org.macports.app/org.macports.app.plist");
    assert_eq!(fs::read(&staged).unwrap(), b"custom plist\0\xff");
    assert_eq!(
        fs::metadata(&staged).unwrap().permissions().mode() & 0o7777,
        0o440
    );
    assert_eq!(fs::metadata(&staged).unwrap().modified().unwrap(), time);
    assert_eq!(
        fs::read_link(base.join("org.macports.app.plist"))
            .unwrap()
            .to_str()
            .unwrap(),
        "/opt/local/etc/LaunchDaemons/org.macports.app/org.macports.app.plist"
    );
    assert!(!temp.path().join("stage/Library").exists());
    assert!(
        s.invoke("portstartupitem::startupitem_create", &[])
            .is_err()
    );
    assert_eq!(fs::read(&staged).unwrap(), b"custom plist\0\xff");
}
#[test]
fn honors_agent_names_and_replaces_an_existing_link_without_following_it() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = session(temp.path());
    s.invoke("startupitem.location", &["LaunchAgents"]).unwrap();
    s.invoke("startupitem.name", &["worker"]).unwrap();
    s.invoke("startupitem.plist", &["worker.plist"]).unwrap();
    let base = temp.path().join("stage/opt/local/etc/LaunchAgents");
    fs::create_dir_all(&base).unwrap();
    let unrelated = temp.path().join("unrelated");
    fs::write(&unrelated, "preserve").unwrap();
    symlink(&unrelated, base.join("worker.plist")).unwrap();
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    assert_eq!(fs::read_to_string(unrelated).unwrap(), "preserve");
    assert_eq!(
        fs::read_link(base.join("worker.plist"))
            .unwrap()
            .to_str()
            .unwrap(),
        "/opt/local/etc/LaunchAgents/org.macports.worker/worker.plist"
    );
}
#[test]
fn rejects_escaping_names_and_linked_directories_without_outside_writes() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = session(temp.path());
    s.invoke("startupitem.uniquename", &["../escape"]).unwrap();
    assert!(
        s.invoke("portstartupitem::startupitem_create", &[])
            .is_err()
    );
    assert_eq!(fs::read_dir(temp.path().join("stage")).unwrap().count(), 0);
    s.invoke("startupitem.uniquename", &["org.macports.app"])
        .unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), temp.path().join("stage/opt")).unwrap();
    assert!(
        s.invoke("portstartupitem::startupitem_create", &[])
            .is_err()
    );
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}
#[test]
fn disabled_items_and_metadata_do_not_stage_files() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = session(temp.path());
    s.invoke("startupitem.type", &["none"]).unwrap();
    s.invoke("startupitem.create", &["yes"]).unwrap();
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    assert_eq!(fs::read_dir(temp.path().join("stage")).unwrap().count(), 0);
    let mut s = Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    assert!(
        s.invoke("portstartupitem::startupitem_create", &[])
            .is_err()
    );
}

#[test]
fn multiple_items_inherit_globals_and_keep_name_based_paths() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = session(temp.path());
    s.invoke("startupitem.uniquename", &["global-name"])
        .unwrap();
    s.invoke("startupitem.plist", &["global.plist"]).unwrap();
    s.invoke(
        "startupitems",
        &[
            "name",
            "first",
            "name",
            "second",
            "location",
            "LaunchAgents",
            "plist",
            "agent.plist",
            "name",
            "disabled",
            "type",
            "none",
        ],
    )
    .unwrap();
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    let root = temp.path().join("stage/opt/local/etc");
    assert_eq!(
        fs::read(root.join("LaunchDaemons/org.macports.first/org.macports.first.plist")).unwrap(),
        b"custom plist\0\xff"
    );
    assert_eq!(
        fs::read_link(root.join("LaunchAgents/agent.plist"))
            .unwrap()
            .to_str()
            .unwrap(),
        "/opt/local/etc/LaunchAgents/org.macports.second/agent.plist"
    );
    assert!(!root.join("LaunchDaemons/org.macports.disabled").exists());
    assert!(!root.join("LaunchDaemons/global-name").exists());
}

#[test]
fn repeated_item_name_merges_attributes_and_resets_derived_names() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = session(temp.path());
    s.invoke(
        "set",
        &[
            "startupitems",
            "name worker location LaunchAgents plist old.plist name worker",
        ],
    )
    .unwrap();
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    let base = temp.path().join("stage/opt/local/etc/LaunchAgents");
    assert!(
        base.join("org.macports.worker/org.macports.worker.plist")
            .is_file()
    );
    assert!(!base.join("org.macports.worker/old.plist").exists());
}

#[test]
fn invalid_later_item_is_rejected_before_any_item_is_written() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = session(temp.path());
    for specification in [
        "custom_file nowhere name worker",
        "name first name second custom_file /definitely/missing/port-rs.plist",
        "name worker plist ../escape",
    ] {
        s.invoke("set", &["startupitems", specification]).unwrap();
        assert!(
            s.invoke("portstartupitem::startupitem_create", &[])
                .is_err()
        );
        assert_eq!(fs::read_dir(temp.path().join("stage")).unwrap().count(), 0);
    }
}

#[test]
fn per_item_create_does_not_change_the_global_default_type() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = session(temp.path());
    s.invoke("unset", &["startupitem.type"]).unwrap();
    s.invoke("set", &["startupitems", "name worker create yes"])
        .unwrap();
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    assert_eq!(fs::read_dir(temp.path().join("stage")).unwrap().count(), 0);
    s.invoke("startupitem.create", &["yes"]).unwrap();
    s.invoke("set", &["system_options(startupitem_type)", "default"])
        .unwrap();
    s.invoke("set", &["os.platform", "darwin"]).unwrap();
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    assert!(
        temp.path()
            .join("stage/opt/local/etc/LaunchDaemons/org.macports.worker/org.macports.worker.plist")
            .is_file()
    );
}
