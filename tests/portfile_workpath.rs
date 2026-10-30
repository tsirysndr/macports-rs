use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
};
fn paths(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let old = root.join("build/_ports_net_app/app");
    let new = root.join("build/new/app");
    let work = new.join("work");
    (old, new, work)
}
fn session(root: &Path, execution: bool) -> Session {
    let (_, new, work) = paths(root);
    let vars = BTreeMap::from([
        (
            "portbuildpath".into(),
            root.join("build/new").to_str().unwrap().into(),
        ),
        ("portpath".into(), "/ports/net/app".into()),
        ("subbuildpath".into(), new.to_str().unwrap().into()),
        ("workpath".into(), work.to_str().unwrap().into()),
        (
            "macports::macportsuser".into(),
            unsafe { libc::getuid() }.to_string(),
        ),
    ]);
    let mut session = Session::open(
        "name app; version 1",
        &vars,
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    if execution {
        session
            .enable_system(&BTreeMap::new(), |_| {
                panic!("work-directory setup must be native")
            })
            .unwrap();
        session.initialize_creation_mask(0o027).unwrap();
    }
    session
}
#[test]
fn fresh_setup_creates_legacy_link_runtime_directories_and_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (old, new, work) = paths(&root);
    let mut session = session(&root, true);
    session.invoke("portutil::create_workpath", &[]).unwrap();
    assert_eq!(fs::read_link(&old).unwrap(), new);
    assert!(work.join(".home").is_dir());
    assert!(work.join(".tmp").is_dir());
    assert_eq!(
        fs::metadata(&work).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert_eq!(
        fs::metadata(work.join(".tmp"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o750
    );
    fs::write(work.join("keep"), "artifact").unwrap();
    let inode = fs::metadata(work.join("keep")).unwrap().ino();
    session.invoke("portutil::create_workpath", &[]).unwrap();
    assert_eq!(fs::metadata(work.join("keep")).unwrap().ino(), inode);
}
#[test]
fn migrates_pre_211_directory_preserving_work_files_and_identity() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (old, new, work) = paths(&root);
    fs::create_dir_all(old.join("work")).unwrap();
    fs::write(old.join("work/artifact"), "compiled").unwrap();
    let inode = fs::metadata(old.join("work/artifact")).unwrap().ino();
    session(&root, true)
        .invoke("portutil::create_workpath", &[])
        .unwrap();
    assert_eq!(fs::read_link(&old).unwrap(), new);
    assert_eq!(fs::metadata(work.join("artifact")).unwrap().ino(), inode);
    assert_eq!(
        fs::read_to_string(work.join("artifact")).unwrap(),
        "compiled"
    );
}
#[test]
fn migrates_older_link_target_and_repairs_broken_links() {
    for broken in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let (old, new, work) = paths(&root);
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        let previous = root.join("previous");
        if !broken {
            fs::create_dir_all(previous.join("work")).unwrap();
            fs::write(previous.join("work/artifact"), "compiled").unwrap();
        }
        symlink(&previous, &old).unwrap();
        session(&root, true)
            .invoke("portutil::create_workpath", &[])
            .unwrap();
        assert_eq!(fs::read_link(&old).unwrap(), new);
        assert!(work.join(".home").is_dir());
        if !broken {
            assert!(!previous.exists());
            assert_eq!(
                fs::read_to_string(work.join("artifact")).unwrap(),
                "compiled"
            );
        }
    }
}
#[test]
fn metadata_cannot_create_or_migrate_work_directories() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    assert!(
        session(&root, false)
            .invoke("portutil::create_workpath", &[])
            .is_err()
    );
    assert!(!root.join("build").exists());
}
