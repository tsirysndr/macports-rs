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

#[test]
fn metadata_queries_preserve_times_sizes_and_link_identity() {
    use std::{
        os::unix::fs::MetadataExt,
        time::{Duration, UNIX_EPOCH},
    };
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("source");
    fs::write(&file, b"hello\0world").unwrap();
    fs::File::open(&file)
        .unwrap()
        .set_times(
            fs::FileTimes::new()
                .set_modified(UNIX_EPOCH + Duration::from_secs(1234567890))
                .set_accessed(UNIX_EPOCH + Duration::from_secs(1234567800)),
        )
        .unwrap();
    let link = root.path().join("link");
    symlink("source", &link).unwrap();
    let broken = root.path().join("broken");
    symlink("missing/../target", &broken).unwrap();
    let mut session = session();
    for path in [&file, &link] {
        for (operation, expected) in [
            ("size", "11"),
            ("mtime", "1234567890"),
            ("atime", "1234567800"),
        ] {
            assert_eq!(
                session
                    .invoke("file", &[operation, path.to_str().unwrap()])
                    .unwrap(),
                expected
            );
        }
    }
    for (path, expected) in [
        (file.as_path(), "file"),
        (link.as_path(), "link"),
        (broken.as_path(), "link"),
        (root.path(), "directory"),
    ] {
        assert_eq!(
            session
                .invoke("file", &["type", path.to_str().unwrap()])
                .unwrap(),
            expected
        );
    }
    assert_eq!(
        session
            .invoke("file", &["readlink", broken.to_str().unwrap()])
            .unwrap(),
        "missing/../target"
    );
    let stat = session
        .invoke("file", &["stat", link.to_str().unwrap()])
        .unwrap();
    let fields = port_rs::tcl_list::parse(&stat).unwrap();
    let values: BTreeMap<_, _> = fields
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| (pair[0].as_str(), pair[1].as_str()))
        .collect();
    let metadata = fs::metadata(&file).unwrap();
    assert_eq!(values["type"], "file");
    assert_eq!(values["size"], "11");
    assert_eq!(values["ino"], metadata.ino().to_string());
    assert_eq!(values["mode"], metadata.mode().to_string());
    assert_eq!(values["mtime"], "1234567890");
    let lstat = session
        .invoke("file", &["lstat", link.to_str().unwrap()])
        .unwrap();
    let fields = port_rs::tcl_list::parse(&lstat).unwrap();
    let values: BTreeMap<_, _> = fields
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| (pair[0].as_str(), pair[1].as_str()))
        .collect();
    assert_eq!(values["type"], "link");
    assert_eq!(values["size"], "6");
}

#[test]
fn stat_populates_callers_array_and_preserves_existing_fields() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("data");
    fs::write(&file, "1234").unwrap();
    let mut session = Session::open(
        r#"
        name app; version 1
        proc inspect {path} {
            set info(existing) retained
            set result [file stat $path info]
            return [list $result $info(type) $info(size) $info(existing)]
        }
        proc scalar_conflict {path} {
            set info scalar
            catch {file stat $path info} message
            return [list $info $message]
        }
    "#,
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    let result = session
        .invoke("inspect", &[file.to_str().unwrap()])
        .unwrap();
    assert_eq!(
        port_rs::tcl_list::parse(&result).unwrap(),
        ["", "file", "4", "retained"]
    );
    let result = session
        .invoke("scalar_conflict", &[file.to_str().unwrap()])
        .unwrap();
    let values = port_rs::tcl_list::parse(&result).unwrap();
    assert_eq!(values[0], "scalar");
    assert!(values[1].contains("isn't array"));
}

#[test]
fn metadata_errors_are_catchable_and_expose_posix_error_codes() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing");
    let file = root.path().join("file");
    fs::write(&file, "data").unwrap();
    let mut session = Session::open(
        r#"
        name app; version 1
        proc failure {operation path} {
            catch {file $operation $path} message options
            return [list $message [dict get $options -errorcode]]
        }
    "#,
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    for (operation, path, code) in [
        ("stat", &missing, "ENOENT"),
        ("readlink", &file, "EINVAL"),
        ("size", &file.join("child"), "ENOTDIR"),
    ] {
        let result = session
            .invoke("failure", &[operation, path.to_str().unwrap()])
            .unwrap();
        let values = port_rs::tcl_list::parse(&result).unwrap();
        let error_code = port_rs::tcl_list::parse(&values[1]).unwrap();
        assert_eq!(error_code[0], "POSIX");
        assert_eq!(error_code[1], code);
        assert!(values[0].contains(path.to_str().unwrap()));
    }
    assert_eq!(
        session
            .invoke("file", &["size", file.to_str().unwrap()])
            .unwrap(),
        "4"
    );
}

#[test]
fn metadata_timestamp_setters_remain_explicitly_unsupported() {
    let error = Session::open(
        "name app; version 1; catch {file mtime /untouched 0}",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("file mtime setter"));
}
