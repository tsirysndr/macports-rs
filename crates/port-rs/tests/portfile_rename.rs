use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{MetadataExt, symlink},
};
fn session(execution: bool) -> Session {
    let mut session = Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    if execution {
        session
            .enable_system(&BTreeMap::new(), |_| {
                panic!("rename must not spawn processes")
            })
            .unwrap();
    }
    session
}
#[test]
fn directory_moves_preserve_contents_and_file_identity() {
    let root = tempfile::tempdir().unwrap();
    let old = root.path().join("old");
    let new = root.path().join("new");
    fs::create_dir_all(old.join("work/deep")).unwrap();
    fs::write(old.join("work/deep/file"), "keep").unwrap();
    let inode = fs::metadata(old.join("work/deep/file")).unwrap().ino();
    session(true)
        .invoke(
            "file",
            &["rename", old.to_str().unwrap(), new.to_str().unwrap()],
        )
        .unwrap();
    assert!(!old.exists());
    assert_eq!(
        fs::read_to_string(new.join("work/deep/file")).unwrap(),
        "keep"
    );
    assert_eq!(
        fs::metadata(new.join("work/deep/file")).unwrap().ino(),
        inode
    );
}
#[test]
fn multiple_sources_move_into_directory_links_and_source_links_remain_links() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("dir");
    fs::create_dir(&dir).unwrap();
    let alias = root.path().join("alias");
    symlink(&dir, &alias).unwrap();
    let one = root.path().join("one");
    let two = root.path().join("two");
    fs::write(&one, "one").unwrap();
    symlink("missing", &two).unwrap();
    session(true)
        .invoke(
            "file",
            &[
                "rename",
                "--",
                one.to_str().unwrap(),
                two.to_str().unwrap(),
                alias.to_str().unwrap(),
            ],
        )
        .unwrap();
    assert_eq!(fs::read_to_string(dir.join("one")).unwrap(), "one");
    assert_eq!(
        fs::read_link(dir.join("two")).unwrap().to_str().unwrap(),
        "missing"
    );
    assert!(fs::symlink_metadata(&two).is_err());
    assert!(
        fs::symlink_metadata(alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}
#[test]
fn force_controls_replacement_and_same_inode_is_a_noop_only_with_force() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("src");
    let dst = root.path().join("dst");
    fs::write(&src, "source").unwrap();
    fs::write(&dst, "destination").unwrap();
    let mut session = session(true);
    let s = src.to_str().unwrap();
    let d = dst.to_str().unwrap();
    assert!(session.invoke("file", &["rename", s, d]).is_err());
    assert_eq!(fs::read_to_string(&dst).unwrap(), "destination");
    session.invoke("file", &["rename", "-force", s, d]).unwrap();
    assert!(!src.exists());
    assert_eq!(fs::read_to_string(&dst).unwrap(), "source");
    assert!(session.invoke("file", &["rename", d, d]).is_err());
    session.invoke("file", &["rename", "-force", d, d]).unwrap();
    fs::hard_link(&dst, &src).unwrap();
    session.invoke("file", &["rename", "-force", s, d]).unwrap();
    assert!(src.exists());
    assert!(dst.exists());
}
#[test]
fn errors_stop_later_operands_and_do_not_replace_nonempty_directories() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("source");
    let dest = root.path().join("destination");
    fs::create_dir(&src).unwrap();
    fs::create_dir_all(dest.join("source")).unwrap();
    fs::write(dest.join("source/keep"), "keep").unwrap();
    let mut session = session(true);
    session
        .invoke("set", &["src", src.to_str().unwrap()])
        .unwrap();
    session
        .invoke("set", &["dst", dest.to_str().unwrap()])
        .unwrap();
    assert_eq!(
        session
            .invoke("catch", &["file rename -force $src $dst", "message"])
            .unwrap(),
        "1"
    );
    assert_eq!(
        session.scalar("errorCode").unwrap(),
        "POSIX EEXIST {file already exists}"
    );
    assert!(src.is_dir());
    assert!(dest.join("source/keep").exists());
    let missing = root.path().join("missing");
    let later = root.path().join("later");
    fs::write(&later, "later").unwrap();
    assert!(
        session
            .invoke(
                "file",
                &[
                    "rename",
                    missing.to_str().unwrap(),
                    later.to_str().unwrap(),
                    dest.to_str().unwrap()
                ]
            )
            .is_err()
    );
    assert!(later.exists());
    assert!(
        session
            .invoke(
                "file",
                &[
                    "rename",
                    src.to_str().unwrap(),
                    later.to_str().unwrap(),
                    missing.to_str().unwrap()
                ]
            )
            .is_err()
    );
    assert!(!missing.exists());
    assert!(session.invoke("file", &["rename"]).is_err());
}
#[test]
fn metadata_cannot_move_files_even_if_the_error_is_caught() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("src");
    let dst = root.path().join("dst");
    fs::write(&src, "keep").unwrap();
    let variables = BTreeMap::from([
        ("src".into(), src.to_str().unwrap().into()),
        ("dst".into(), dst.to_str().unwrap().into()),
    ]);
    assert!(
        Session::open(
            "name app; version 1; catch {file rename $src $dst}",
            &variables,
            &PortSpec::named("app"),
            PortGroups::default()
        )
        .is_err()
    );
    assert!(src.exists());
    assert!(!dst.exists());
}

#[test]
fn force_rejects_type_mismatches_and_moves_into_self() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("node");
    let target = root.path().join("target");
    fs::write(&source, "keep").unwrap();
    fs::create_dir_all(target.join("node")).unwrap();
    let mut session = session(true);
    let error = session
        .invoke(
            "file",
            &[
                "rename",
                "-force",
                source.to_str().unwrap(),
                target.to_str().unwrap(),
            ],
        )
        .unwrap_err();
    assert!(error.to_string().contains("can't overwrite directory"));
    session
        .invoke("set", &["src", source.to_str().unwrap()])
        .unwrap();
    session
        .invoke("set", &["dst", target.to_str().unwrap()])
        .unwrap();
    session
        .invoke("catch", &["file rename -force $src $dst"])
        .unwrap();
    assert_eq!(session.scalar("errorCode").unwrap(), "NONE");
    assert_eq!(fs::read_to_string(&source).unwrap(), "keep");
    let dir = root.path().join("directory");
    fs::create_dir(&dir).unwrap();
    let error = session
        .invoke(
            "file",
            &[
                "rename",
                "-force",
                dir.to_str().unwrap(),
                source.to_str().unwrap(),
            ],
        )
        .unwrap_err();
    assert!(error.to_string().contains("can't overwrite file"));
    let error = session
        .invoke(
            "file",
            &[
                "rename",
                dir.to_str().unwrap(),
                dir.join("inside").to_str().unwrap(),
            ],
        )
        .unwrap_err();
    assert!(error.to_string().contains("move a directory into itself"));
    assert!(dir.is_dir());
}
