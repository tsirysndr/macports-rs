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

fn session() -> Session {
    Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap()
}
fn query(session: &mut Session, op: &str, path: &Path) -> bool {
    match session
        .invoke("file", &[op, path.to_str().unwrap()])
        .unwrap()
        .as_str()
    {
        "1" => true,
        "0" => false,
        value => panic!("non-boolean file result: {value}"),
    }
}

#[test]
fn file_predicates_follow_links_and_handle_missing_or_invalid_paths() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("source with spaces.c");
    fs::write(&file, "int main(void) { return 0; }").unwrap();
    let link = root.path().join("source-link");
    symlink(&file, &link).unwrap();
    let directory_link = root.path().join("directory-link");
    symlink(root.path(), &directory_link).unwrap();
    let broken = root.path().join("broken");
    symlink("missing", &broken).unwrap();
    let loop_link = root.path().join("loop");
    symlink("loop", &loop_link).unwrap();
    let mut session = session();
    for path in [&file, &link] {
        assert!(query(&mut session, "exists", path));
        assert!(query(&mut session, "isfile", path));
        assert!(!query(&mut session, "isdirectory", path));
        assert!(query(&mut session, "owned", path));
    }
    assert!(query(&mut session, "isdirectory", &directory_link));
    assert!(!query(&mut session, "isfile", &directory_link));
    for path in [
        &broken,
        &loop_link,
        &root.path().join("missing"),
        Path::new(""),
        Path::new("invalid\0path"),
    ] {
        for op in [
            "exists",
            "isfile",
            "isdirectory",
            "readable",
            "writable",
            "executable",
            "owned",
        ] {
            assert!(!query(&mut session, op, path), "{op}: {path:?}");
        }
    }
}

#[test]
fn access_checks_permissions_without_executing_the_file() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("compiler");
    // Deliberately not a valid program: executable means permission, not a
    // successful launch or a known compiler version.
    fs::write(&file, "not an executable format").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    let mut session = session();
    assert!(query(&mut session, "readable", &file));
    assert!(query(&mut session, "writable", &file));
    assert!(!query(&mut session, "executable", &file));
    fs::set_permissions(&file, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(query(&mut session, "executable", &file));
    // No cache: compiler installation/removal must be visible to later checks.
    fs::remove_file(&file).unwrap();
    assert!(!query(&mut session, "exists", &file));
    assert!(!query(&mut session, "executable", &file));
}

#[test]
fn metadata_can_select_dependencies_using_host_file_probes() {
    let root = tempfile::tempdir().unwrap();
    let compiler = root.path().join("clang");
    fs::write(&compiler, "compiler fixture").unwrap();
    fs::set_permissions(&compiler, fs::Permissions::from_mode(0o700)).unwrap();
    let mut variables = BTreeMap::new();
    variables.insert(
        "compiler_candidate".to_owned(),
        compiler.to_str().unwrap().to_owned(),
    );
    let mut session = Session::open(
        r#"
        name app; version 1
        if {[file executable $compiler_candidate]} {
            depends_build port:compiler-present
        } else {
            depends_build port:compiler-fallback
        }
    "#,
        &variables,
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    assert_eq!(
        session.scalar("depends_build").unwrap(),
        "port:compiler-present"
    );
    assert!(
        session
            .invoke("file", &["exists"])
            .unwrap_err()
            .to_string()
            .contains("wrong # args")
    );
    assert!(query(&mut session, "exists", &compiler));
}
