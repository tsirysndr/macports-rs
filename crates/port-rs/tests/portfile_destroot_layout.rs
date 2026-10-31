use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{collections::BTreeMap, fs, os::unix::fs::symlink, path::Path};
fn session(root: &Path, execution: bool) -> Session {
    let mut s = Session::open(
        "name app; version 1",
        &BTreeMap::from([
            ("destroot".into(), root.to_str().unwrap().into()),
            ("prefix".into(), "/opt/local".into()),
            ("applications_dir".into(), "/Applications/MacPorts".into()),
            ("frameworks_dir".into(), "/Library/Frameworks".into()),
        ]),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    if execution {
        s.enable_system(&BTreeMap::new(), |_| panic!("layout checks must be native"))
            .unwrap();
    }
    s
}
fn file(root: &Path, name: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "preserve").unwrap();
}
#[test]
fn accepts_prefix_subtrees_and_platform_installation_directories() {
    let root = tempfile::tempdir().unwrap();
    for directory in [
        "bin",
        "etc",
        "include",
        "lib",
        "libexec",
        "sbin",
        "share",
        "src",
        "var",
        "www",
        "Applications",
        "Developer",
        "Library",
    ] {
        file(root.path(), &format!("opt/local/{directory}/nested/.file"));
    }
    for path in [
        "Applications/MacPorts/App.app/file",
        "Library/Frameworks/App.framework/file",
        "Library/LaunchAgents/app.plist",
        "Library/LaunchDaemons/app.plist",
        "Library/StartupItems/app/file",
    ] {
        file(root.path(), path);
    }
    let result = session(root.path(), true).check_destroot_layout().unwrap();
    assert!(result.violations.is_empty());
    assert!(!result.override_enabled);
}
#[test]
fn collects_all_layout_warnings_including_hidden_and_prefix_lookalikes() {
    let root = tempfile::tempdir().unwrap();
    for path in [
        "opt/local/unexpected/file",
        "opt/local/.hidden",
        "opt/local-extra/file",
        "Applications/Unapproved/file",
        "Library/Unknown/file",
        ".root-hidden",
        "usr/bin/app",
    ] {
        file(root.path(), path);
    }
    let mut result = session(root.path(), true).check_destroot_layout().unwrap();
    result.violations.sort();
    assert_eq!(
        result.violations,
        [
            "/.root-hidden",
            "/Applications/Unapproved",
            "/Library/Unknown",
            "/opt/local-extra",
            "/opt/local/.hidden",
            "/opt/local/unexpected",
            "/usr"
        ]
    );
    assert_eq!(
        fs::read_to_string(root.path().join("opt/local/.hidden")).unwrap(),
        "preserve"
    );
}
#[test]
fn override_requires_literal_yes_and_skips_inspection() {
    let root = tempfile::tempdir().unwrap();
    file(root.path(), "rogue");
    let mut s = session(root.path(), true);
    s.invoke("destroot.violate_mtree", &["true"]).unwrap();
    assert_eq!(s.check_destroot_layout().unwrap().violations, ["/rogue"]);
    s.invoke("destroot.violate_mtree", &["yes"]).unwrap();
    // The override branch must not require unused path configuration.
    s.invoke(
        "unset",
        &["destroot", "prefix", "applications_dir", "frameworks_dir"],
    )
    .unwrap();
    let result = s.check_destroot_layout().unwrap();
    assert!(result.override_enabled);
    assert!(result.violations.is_empty());
}
#[test]
fn checks_linked_prefix_children_and_does_not_follow_permitted_subtrees() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    file(outside.path(), "unexpected");
    fs::create_dir(root.path().join("opt")).unwrap();
    symlink(outside.path(), root.path().join("opt/local")).unwrap();
    symlink("/missing-target", outside.path().join("bin")).unwrap();
    assert_eq!(
        session(root.path(), true)
            .check_destroot_layout()
            .unwrap()
            .violations,
        ["/opt/local/unexpected"]
    );
    assert_eq!(
        fs::read_to_string(outside.path().join("unexpected")).unwrap(),
        "preserve"
    );
}
#[test]
fn absent_stage_is_not_a_layout_error_and_metadata_execution_is_denied() {
    let root = tempfile::tempdir().unwrap();
    assert!(
        session(&root.path().join("missing"), true)
            .check_destroot_layout()
            .unwrap()
            .violations
            .is_empty()
    );
    assert!(session(root.path(), false).check_destroot_layout().is_err());
}
