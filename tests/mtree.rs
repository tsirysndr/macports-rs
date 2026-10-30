use port_rs::build::mtree::DirectorySpec;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

#[test]
fn creates_hierarchy_updates_modes_and_preserves_extras() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("bin")).unwrap();
    fs::write(temp.path().join("bin/app"), b"payload").unwrap();
    fs::set_permissions(temp.path().join("bin"), fs::Permissions::from_mode(0o700)).unwrap();
    let spec = DirectorySpec::parse("# hierarchy\n/set type=dir uname=root gname=wheel mode=0755\n.\nbin mode=0750\n..\nshare\nman\nman1\n..\n..\n..\n..\n").unwrap();
    for _ in 0..2 {
        spec.apply_unprivileged(temp.path()).unwrap();
    }
    assert!(temp.path().join("share/man/man1").is_dir());
    assert_eq!(fs::read(temp.path().join("bin/app")).unwrap(), b"payload");
    assert_eq!(
        fs::metadata(temp.path().join("bin"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o750
    );
    assert_eq!(
        fs::metadata(temp.path().join("share/man"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o755
    );
}
#[test]
fn defaults_and_multiple_root_blocks_use_last_mode() {
    let temp = tempfile::tempdir().unwrap();
    let spec = DirectorySpec::parse("/set type=dir mode=0700\n.\nusr\n..\n..\n/unset mode\n/set mode=0755\n.\nLibrary\n..\n..\n").unwrap();
    spec.apply_unprivileged(temp.path()).unwrap();
    assert_eq!(
        fs::metadata(temp.path()).unwrap().permissions().mode() & 0o7777,
        0o755
    );
    assert_eq!(
        fs::metadata(temp.path().join("usr"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o700
    );
}
#[test]
fn rejects_links_before_any_mode_changes_or_creation() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), temp.path().join("share")).unwrap();
    let before = fs::metadata(temp.path()).unwrap().permissions().mode();
    let spec = DirectorySpec::parse("/set type=dir mode=0755\n.\nbin\n..\nshare\nman\n..\n..\n..")
        .unwrap();
    assert!(spec.apply_unprivileged(temp.path()).is_err());
    assert!(!temp.path().join("bin").exists());
    assert!(!outside.path().join("man").exists());
    assert_eq!(
        fs::metadata(temp.path()).unwrap().permissions().mode(),
        before
    );
}
#[test]
fn unsupported_or_malformed_specs_fail_during_parsing() {
    for source in [
        "",
        "..",
        "/set type=file mode=0644\n.",
        "/set type=dir mode=0755\n.\n../escape",
        "/set type=dir mode=0755\n.\n/escape",
        "/set type=dir mode=0755\n.\nfoo\\040bar",
        "/set type=dir mode=888\n.",
        "/set type=dir mode=0755 time=0\n.",
        "/set type=dir mode=0755\n.\n*",
        "/set type=dir\n.",
    ] {
        assert!(DirectorySpec::parse(source).is_err(), "{source}");
    }
}

#[test]
fn session_prepares_platform_and_prefix_from_installed_specs_without_subprocesses() {
    use port_rs::{
        portfile::{PortGroups, Session},
        spec::PortSpec,
    };
    use std::collections::BTreeMap;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("stage");
    let share = temp.path().join("share");
    fs::create_dir(&root).unwrap();
    fs::create_dir_all(share.join("install")).unwrap();
    fs::write(share.join("install/prefix.mtree"), "/set type=dir uname=root gname=admin mode=0755\n.\nbin\n..\nshare\nman\nman1\n..\n..\n..\n..\n").unwrap();
    fs::write(share.join("install/macosx.mtree"), "/set type=dir uname=root gname=wheel mode=0755\n.\nusr\n..\n..\n/set gname=admin\n.\nApplications\n..\nLibrary\n..\n..\n").unwrap();
    let vars = BTreeMap::from([
        ("destroot".into(), root.to_str().unwrap().into()),
        ("portsharepath".into(), share.to_str().unwrap().into()),
        ("prefix".into(), "/opt/local".into()),
        ("os.platform".into(), "darwin".into()),
        ("applications_dir".into(), "/Applications/MacPorts".into()),
        (
            "frameworks_dir".into(),
            "/opt/local/Library/Frameworks".into(),
        ),
    ]);
    let mut s = Session::open(
        "name app; version 1",
        &vars,
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    assert!(s.prepare_destroot_hierarchy().is_err());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    s.enable_system(&BTreeMap::new(), |_| panic!("no external mtree"))
        .unwrap();
    s.initialize_creation_mask(0o022).unwrap();
    s.prepare_destroot_hierarchy().unwrap();
    for path in [
        "usr",
        "Applications/MacPorts",
        "opt/local/bin",
        "opt/local/share/man/man1",
        "opt/local/Library/Frameworks",
    ] {
        assert!(root.join(path).is_dir(), "{path}");
    }
    // A hierarchy path cannot escape via a symlink, even on repeated preparation.
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), root.join("escape")).unwrap();
    s.invoke("set", &["prefix", "/escape/local"]).unwrap();
    assert!(s.prepare_destroot_hierarchy().is_err());
    assert!(!outside.path().join("local").exists());
}

#[test]
fn restrictive_repeated_root_is_updated_after_all_descendants() {
    let temp = tempfile::tempdir().unwrap();
    let spec = DirectorySpec::parse(
        "/set type=dir mode=0755\n.\nfirst\n..\n..\n. mode=0000\nlast\n..\n..",
    )
    .unwrap();
    let result = spec.apply_unprivileged(temp.path());
    let mode = fs::metadata(temp.path()).unwrap().permissions().mode() & 0o7777;
    // Restore access even when the implementation under test fails halfway.
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    result.unwrap();
    assert_eq!(mode, 0);
    for name in ["first", "last"] {
        assert_eq!(
            fs::metadata(temp.path().join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o755
        );
    }
}
