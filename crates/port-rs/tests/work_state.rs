use port_rs::build::{
    state::{Compatibility, StateFile},
    work_state::prepare,
};
use std::{
    fs,
    os::unix::fs::{MetadataExt, symlink},
    time::{Duration, SystemTime},
};

#[test]
fn fresh_and_repeated_open_preserve_completed_targets() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("Portfile");
    fs::write(&source, "name app\nversion 1\n").unwrap();
    let work = root.path().join("work");
    fs::create_dir(&work).unwrap();
    let mut first = prepare(&work, "app", &source, Some(0o027), false).unwrap();
    assert!(first.initialized);
    assert_eq!(first.state.first("version").unwrap().as_deref(), Some("2"));
    first.state.record("target", "org.macports.build").unwrap();
    drop(first);
    let mut second = prepare(&work, "app", &source, None, false).unwrap();
    assert!(!second.initialized);
    assert_eq!(second.compatibility, Compatibility::Current);
    assert!(
        second
            .state
            .contains("target", "org.macports.build")
            .unwrap()
    );
}

#[test]
fn changed_portfile_cleans_work_without_following_links_or_replacing_lock() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("Portfile");
    fs::write(&source, "version 1").unwrap();
    let work = root.path().join("work");
    fs::create_dir(&work).unwrap();
    drop(prepare(&work, "app", &source, None, false).unwrap());
    let path = work.join(".macports.app.state");
    let inode = fs::metadata(&path).unwrap().ino();
    fs::create_dir_all(work.join("src/nested")).unwrap();
    fs::write(work.join("src/nested/object"), "old").unwrap();
    symlink(root.path(), work.join("external")).unwrap();
    fs::write(&source, "version 2").unwrap();
    let mut next = prepare(&work, "app", &source, None, false).unwrap();
    assert_eq!(next.compatibility, Compatibility::PortfileChanged);
    assert!(next.initialized);
    assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
    assert!(StateFile::try_open(&path, None).unwrap().is_none());
    assert_eq!(fs::read_dir(&work).unwrap().count(), 1);
    assert_eq!(fs::read_to_string(&source).unwrap(), "version 2");
    assert_eq!(next.state.first("version").unwrap().as_deref(), Some("2"));
    drop(next);
    assert!(
        !prepare(&work, "app", &source, None, false)
            .unwrap()
            .initialized
    );
}

#[test]
fn unknown_versions_and_ignore_different_preserve_artifacts() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("Portfile");
    fs::write(&source, "version 1").unwrap();
    let work = root.path().join("work");
    fs::create_dir(&work).unwrap();
    let state = work.join(".macports.app.state");
    fs::write(&state, "version: 42\n").unwrap();
    fs::write(work.join("artifact"), "keep").unwrap();
    let opened = prepare(&work, "app", &source, None, false).unwrap();
    assert_eq!(
        opened.compatibility,
        Compatibility::UnsupportedVersion("42".into())
    );
    assert!(!opened.initialized);
    drop(opened);
    fs::write(&state, "version: 2\nchecksum: obsolete\n").unwrap();
    let opened = prepare(&work, "app", &source, None, true).unwrap();
    assert!(!opened.initialized);
    assert!(work.join("artifact").exists());
    assert_eq!(
        fs::read_to_string(state).unwrap(),
        "version: 2\nchecksum: obsolete\n"
    );
}

#[test]
fn future_portfile_and_invalid_names_fail_before_mutation() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("Portfile");
    fs::write(&source, "version 1").unwrap();
    let work = root.path().join("work");
    fs::create_dir(&work).unwrap();
    assert!(prepare(&work, "../escape", &source, None, false).is_err());
    assert_eq!(fs::read_dir(&work).unwrap().count(), 0);
    drop(prepare(&work, "app", &source, None, false).unwrap());
    let state = work.join(".macports.app.state");
    let before = fs::read(&state).unwrap();
    fs::File::open(&source)
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(3600))
        .unwrap();
    let error = prepare(&work, "app", &source, None, true).err().unwrap();
    assert!(error.to_string().contains("from the future"));
    assert_eq!(fs::read(state).unwrap(), before);
}

#[test]
fn dry_run_reads_locked_state_without_locking_or_invalidating() {
    use port_rs::build::{state::LockStatus, work_state::prepare_dry_run};
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("Portfile");
    fs::write(&source, "version 1").unwrap();
    let work = root.path().join("work");
    fs::create_dir(&work).unwrap();
    let mut active = prepare(&work, "app", &source, None, false).unwrap();
    active.state.record("target", "org.macports.fetch").unwrap();
    fs::write(&source, "version 2").unwrap();
    let path = work.join(".macports.app.state");
    let before = fs::read(&path).unwrap();
    let mut dry = prepare_dry_run(&work, "app", &source, false).unwrap();
    assert_eq!(dry.compatibility, Compatibility::PortfileChanged);
    assert_eq!(dry.state.lock_status, LockStatus::NotRequested);
    assert!(dry.state.contains("target", "org.macports.fetch").unwrap());
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(StateFile::try_open(&path, None).unwrap().is_none());
}
