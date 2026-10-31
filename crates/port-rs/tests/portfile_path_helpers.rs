use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{collections::BTreeMap, fs, os::unix::fs::symlink};
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
fn shellescape_matches_macports_and_preserves_shell_metacharacters_as_data() {
    let mut session = session();
    for command in ["shellescape", "macports::shellescape"] {
        assert_eq!(
            session.invoke(command, &["azAZ09.:@%/+=_-"]).unwrap(),
            "azAZ09.:@%/+=_-"
        );
        assert_eq!(session.invoke(command, &[""]).unwrap(), "");
        assert_eq!(
            session.invoke(command, &["a b'\"$\\\n"]).unwrap(),
            "a\\ b\\'\\\"\\$\\\\\\\n"
        );
        assert_eq!(session.invoke(command, &["é"]).unwrap(), "\\é");
        for input in [
            "path with spaces/file",
            "$(printf expanded); & | > < * ? [x] {x}",
            "quotes'\"and\\backslash",
            "é日本語",
        ] {
            let quoted = session.invoke(command, &[input]).unwrap();
            let output = std::process::Command::new("/bin/sh")
                .args(["-c", &format!("printf '%s' {quoted}")])
                .output()
                .unwrap();
            assert!(output.status.success());
            assert_eq!(output.stdout, input.as_bytes());
        }
    }
}
#[test]
fn realpath_follows_links_and_errors_on_missing_files_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("actual/deep")).unwrap();
    let file = root.path().join("actual/file");
    fs::write(&file, "data").unwrap();
    let link = root.path().join("link");
    symlink("actual/deep", &link).unwrap();
    let mut session = session();
    let resolved = session
        .invoke("realpath", &[link.join("../file").to_str().unwrap()])
        .unwrap();
    assert_eq!(resolved, fs::canonicalize(&file).unwrap().to_str().unwrap());
    symlink("missing", root.path().join("broken")).unwrap();
    for path in [root.path().join("missing"), root.path().join("broken")] {
        session
            .invoke("set", &["path", path.to_str().unwrap()])
            .unwrap();
        assert_eq!(
            session
                .invoke("catch", &["realpath $path", "message"])
                .unwrap(),
            "1"
        );
        assert!(
            session
                .scalar("message")
                .unwrap()
                .starts_with("realpath failed: ")
        );
        assert_eq!(session.scalar("errorCode").unwrap(), "NONE");
    }
    assert!(!root.path().join("missing").exists());
}
#[test]
fn old_build_paths_use_global_configuration_and_ordered_url_mapping() {
    let mut session = session();
    session
        .invoke(
            "set",
            &["portbuildpath", "/var/macports/build/new-location"],
        )
        .unwrap();
    for (source, suffix) in [
        ("/opt/ports/net/curl", "_opt_ports_net_curl"),
        (
            "https://example.org/ports/net/curl",
            "https.example.org_ports_net_curl",
        ),
        ("file:///ports/curl", "file._ports_curl"),
    ] {
        session.invoke("set", &["portpath", source]).unwrap();
        assert_eq!(
            session.invoke("portutil::get_oldbuildpath", &[]).unwrap(),
            format!("/var/macports/build/{suffix}")
        );
    }
    session
        .invoke(
            "proc",
            &[
                "inside",
                "",
                "set portpath local; set portbuildpath local; portutil::get_oldbuildpath",
            ],
        )
        .unwrap();
    assert_eq!(
        session.invoke("inside", &[]).unwrap(),
        "/var/macports/build/file._ports_curl"
    );
}
