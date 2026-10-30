use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, path::Path};
fn tool(dir: &Path, name: &str) -> String {
    let path = dir.join(name);
    fs::write(&path, "fixture, never executed").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path.to_str().unwrap().to_owned()
}
fn session(path: &str) -> Session {
    let mut session = Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    session
        .enable_system(&BTreeMap::from([("PATH".into(), path.into())]), |_| {
            panic!("lookup must not execute tools")
        })
        .unwrap();
    session
}
#[test]
fn hints_precede_path_and_stale_hints_fall_back_without_host_path_leakage() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first");
    let second = root.path().join("second");
    fs::create_dir(&first).unwrap();
    fs::create_dir(&second).unwrap();
    let primary = tool(&first, "make");
    let hinted = tool(&second, "make");
    let mut session = session(&format!("{}:{}", first.display(), second.display()));
    assert_eq!(
        session.invoke("findBinary", &["make", &hinted]).unwrap(),
        hinted
    );
    assert_eq!(
        session
            .invoke("macports::findBinary", &["make", "/missing/hint"])
            .unwrap(),
        primary
    );
    fs::set_permissions(first.join("make"), fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        session.invoke("macports::binaryInPath", &["make"]).unwrap(),
        hinted
    );
    session
        .invoke("set", &["env(PATH)", "/missing/search/path"])
        .unwrap();
    assert!(
        session
            .invoke("findBinary", &["sh"])
            .unwrap_err()
            .to_string()
            .contains("MacPorts configuration time location")
    );
    assert_eq!(
        session.invoke("findBinary", &["unused", &hinted]).unwrap(),
        hinted
    );
}
#[test]
fn make_selection_matches_build_type_and_platform_and_preserves_overrides() {
    let root = tempfile::tempdir().unwrap();
    for name in ["make", "bsdmake", "pmake", "gnumake", "gmake"] {
        tool(root.path(), name);
    }
    let mut session = session(root.path().to_str().unwrap());
    for (kind, platform, expected) in [
        ("default", "darwin", "make"),
        ("bsd", "darwin", "bsdmake"),
        ("bsd", "freebsd", "make"),
        ("bsd", "other", "pmake"),
        ("gnu", "darwin", "gnumake"),
        ("gnu", "linux", "make"),
        ("gnu", "freebsd", "gmake"),
        ("unknown", "darwin", "gnumake"),
    ] {
        session.invoke("set", &["build.type", kind]).unwrap();
        session.invoke("set", &["os.platform", platform]).unwrap();
        assert_eq!(
            session.scalar("build.cmd").unwrap(),
            root.path().join(expected).to_str().unwrap()
        );
    }
    let hint = tool(root.path(), "configured-make");
    session
        .invoke("set", &["::portutil::autoconf::gnumake_path", &hint])
        .unwrap();
    session.invoke("set", &["build.type", "gnu"]).unwrap();
    assert_eq!(session.scalar("build.cmd").unwrap(), hint);
    session
        .invoke("set", &["build.cmd", "custom --flag"])
        .unwrap();
    session.invoke("set", &["build.type", "bsd"]).unwrap();
    assert_eq!(session.scalar("build.cmd").unwrap(), "custom --flag");
}
#[test]
fn xcode_uses_the_configured_command_and_missing_tools_stop_before_execution() {
    let mut session = session("/missing/search/path");
    session.invoke("set", &["os.platform", "darwin"]).unwrap();
    session.invoke("set", &["build.type", "xcode"]).unwrap();
    session
        .invoke("set", &["xcodebuildcmd", "/configured/xcodebuild"])
        .unwrap();
    assert_eq!(
        session.scalar("build.cmd").unwrap(),
        "/configured/xcodebuild"
    );
    session.invoke("set", &["xcodebuildcmd", "none"]).unwrap();
    assert_eq!(session.scalar("build.cmd").unwrap(), "");
    session.invoke("set", &["build.type", "default"]).unwrap();
    assert!(
        session
            .invoke("portbuild::build_main", &[])
            .unwrap_err()
            .to_string()
            .contains("No build command found")
    );
    session.invoke("set", &["build.type", "pbx"]).unwrap();
    session.invoke("set", &["os.platform", "freebsd"]).unwrap();
    assert_eq!(session.scalar("build.cmd").unwrap(), "");
}
