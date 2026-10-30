use port_rs::{
    portfile::{LibtoolCleanup, PortGroups, Session},
    spec::PortSpec,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::Path,
};
fn session(root: &Path, delete: bool, execution: bool) -> Session {
    let mut s = Session::open(
        "name app; version 1",
        &BTreeMap::from([
            ("destroot".into(), root.to_str().unwrap().into()),
            ("destroot.delete_la_files".into(), delete.to_string()),
        ]),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    if execution {
        s.enable_system(&BTreeMap::new(), |_| {
            panic!("libtool cleanup must be native")
        })
        .unwrap();
    }
    s
}
const ARCHIVE: &str = "# libthing.la - a libtool library file\ndependency_libs=' -L/opt/local/lib -lother'\ndlname='libthing.dylib'\n";
#[test]
fn clears_dependency_lists_preserving_mode_inode_hardlinks_and_other_fields() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("libthing.la");
    fs::write(&path, ARCHIVE).unwrap();
    fs::hard_link(&path, root.path().join("alias")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let before = fs::metadata(&path).unwrap();
    let mut s = session(root.path(), false, true);
    assert_eq!(
        s.clean_destroot_libtool().unwrap(),
        LibtoolCleanup {
            removed: 0,
            rewritten: 1
        }
    );
    let expected =
        "# libthing.la - a libtool library file\ndependency_libs=''\ndlname='libthing.dylib'\n";
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    assert_eq!(
        fs::read_to_string(root.path().join("alias")).unwrap(),
        expected
    );
    let after = fs::metadata(&path).unwrap();
    assert_eq!(before.ino(), after.ino());
    assert_eq!(before.uid(), after.uid());
    assert_eq!(before.gid(), after.gid());
    assert_eq!(after.permissions().mode() & 0o7777, 0o444);
    assert_eq!(
        s.clean_destroot_libtool().unwrap(),
        LibtoolCleanup::default()
    );
}
#[test]
fn deletion_recognizes_relative_and_staged_absolute_links_before_removing_targets() {
    let root = tempfile::tempdir().unwrap();
    let lib = root.path().join("opt/local/lib");
    fs::create_dir_all(&lib).unwrap();
    fs::write(lib.join("a.la"), ARCHIVE).unwrap();
    symlink("a.la", lib.join("z-relative.la")).unwrap();
    symlink("/opt/local/lib/a.la", lib.join("z-absolute.la")).unwrap();
    symlink("missing.la", lib.join("broken.la")).unwrap();
    let result = session(root.path(), true, true)
        .clean_destroot_libtool()
        .unwrap();
    assert_eq!(
        result,
        LibtoolCleanup {
            removed: 3,
            rewritten: 0
        }
    );
    assert!(fs::symlink_metadata(lib.join("broken.la")).is_ok());
    assert_eq!(fs::read_dir(lib).unwrap().count(), 1);
}
#[test]
fn retains_non_libtool_archives_and_never_rewrites_link_targets_outside_stage() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("external.la"), ARCHIVE).unwrap();
    symlink(outside.path(), root.path().join("outside-directory")).unwrap();
    // A relative symlink can point outside the stage. Keep mode must not edit it.
    assert_eq!(root.path().parent(), outside.path().parent());
    symlink(
        Path::new("..")
            .join(outside.path().file_name().unwrap())
            .join("external.la"),
        root.path().join("external.la"),
    )
    .unwrap();
    fs::write(
        root.path().join("ordinary.la"),
        "not libtool\ndependency_libs='keep'\n",
    )
    .unwrap();
    fs::write(
        root.path().join("late-marker.la"),
        "not libtool\na libtool library file\n",
    )
    .unwrap();
    let mut s = session(root.path(), false, true);
    assert_eq!(
        s.clean_destroot_libtool().unwrap(),
        LibtoolCleanup::default()
    );
    assert_eq!(
        fs::read_to_string(outside.path().join("external.la")).unwrap(),
        ARCHIVE
    );
    assert_eq!(
        fs::read_to_string(root.path().join("ordinary.la")).unwrap(),
        "not libtool\ndependency_libs='keep'\n"
    );
    assert!(root.path().join("late-marker.la").exists());
}
#[test]
fn replacements_follow_sed_greedy_quotes_and_metadata_cannot_mutate() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("example.la");
    let bytes = b"# a libtool library file\n# dependency_libs 'first' middle 'last' trailing\ndependency_libs=unquoted\nother='keep'\n\xff\n";
    fs::write(&path, bytes).unwrap();
    assert!(
        session(root.path(), false, false)
            .clean_destroot_libtool()
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    session(root.path(), false, true)
        .clean_destroot_libtool()
        .unwrap();
    assert_eq!(fs::read(path).unwrap(), b"# a libtool library file\n# dependency_libs '' trailing\ndependency_libs=unquoted\nother='keep'\n\xff\n");
}
