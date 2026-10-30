use port_rs::{
    build::state::StateFile,
    portfile::{PortGroups, Session, TargetOutcome},
    spec::PortSpec,
};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};
fn setup(root: &Path) -> (Session, PathBuf) {
    let stage = root.join("stage");
    fs::create_dir(&stage).unwrap();
    let source = "name app; version 1; destroot {}";
    fs::write(root.join("Portfile"), source).unwrap();
    let vars = BTreeMap::from([
        ("destroot".into(), stage.to_str().unwrap().into()),
        ("prefix".into(), "/opt/local".into()),
        ("applications_dir".into(), "/Applications/MacPorts".into()),
        ("frameworks_dir".into(), "/Library/Frameworks".into()),
        ("destroot.delete_la_files".into(), "no".into()),
        ("portpath".into(), root.to_str().unwrap().into()),
        (
            "portdbpath".into(),
            root.join("db").to_str().unwrap().into(),
        ),
        (
            "portbuildpath".into(),
            root.join("build/new").to_str().unwrap().into(),
        ),
        (
            "subbuildpath".into(),
            root.join("build/new/app").to_str().unwrap().into(),
        ),
        (
            "workpath".into(),
            root.join("build/new/app/work").to_str().unwrap().into(),
        ),
        ("configure.developer_dir".into(), "".into()),
    ]);
    let mut s = Session::open(
        source,
        &vars,
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    s.enable_system(&BTreeMap::new(), |_| panic!("finish needs no subprocess"))
        .unwrap();
    s.initialize_creation_mask(0o027).unwrap();
    s.invoke("set", &["portdestroot::oldmask", "0027"]).unwrap();
    s.invoke("umask", &["0077"]).unwrap();
    (s, stage)
}
fn put(root: &Path, name: &str, content: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}
#[test]
fn finish_composes_cleanup_pruning_compression_layout_and_mask_restoration() {
    let temp = tempfile::tempdir().unwrap();
    let (mut s, stage) = setup(temp.path());
    put(&stage, "opt/local/share/info/dir", "stray");
    put(&stage, "opt/local/lib/charset.alias", "stray");
    put(
        &stage,
        "opt/local/lib/example.la",
        "# a libtool library file\ndependency_libs=' -lold'\n",
    );
    put(&stage, "opt/local/share/man/man1/app.1", "manual text");
    fs::create_dir_all(stage.join("empty/deep")).unwrap();
    s.invoke("destroot.keepdirs", &["/opt/local/var/keep"])
        .unwrap();
    assert_eq!(
        s.invoke("portdestroot::destroot_finish", &["ignored"])
            .unwrap(),
        "0"
    );
    assert!(!stage.join("opt/local/share/info/dir").exists());
    assert!(!stage.join("opt/local/lib/charset.alias").exists());
    assert_eq!(
        fs::read_to_string(stage.join("opt/local/lib/example.la")).unwrap(),
        "# a libtool library file\ndependency_libs=''\n"
    );
    assert!(!stage.join("empty").exists());
    assert!(stage.join("opt/local/var/keep/.turd_app").exists());
    let mut text = String::new();
    flate2::read::GzDecoder::new(
        fs::File::open(stage.join("opt/local/share/man/man1/app.1.gz")).unwrap(),
    )
    .read_to_string(&mut text)
    .unwrap();
    assert_eq!(text, "manual text");
    assert_eq!(s.invoke("umask", &["0027"]).unwrap(), "0027");
}
#[test]
fn empty_stage_failure_restores_mask_and_does_not_record_completion() {
    let temp = tempfile::tempdir().unwrap();
    let (mut s, stage) = setup(temp.path());
    put(&stage, "opt/local/share/info/dir", "stray only");
    // The host supplies preparation until native destroot_start is implemented.
    s.invoke("proc", &["portdestroot::destroot_start", "args", ""])
        .unwrap();
    let item = s.scalar("org.macports.destroot").unwrap();
    assert!(
        s.run_prepared_target(&item, false)
            .unwrap_err()
            .to_string()
            .contains("Staging app into destroot failed")
    );
    assert_eq!(s.invoke("umask", &["0027"]).unwrap(), "0027");
    let mut state = StateFile::open(
        &temp.path().join("build/new/app/work/.macports.app.state"),
        None,
    )
    .unwrap();
    assert!(!state.contains("target", "org.macports.destroot").unwrap());
}
#[test]
fn prepared_target_records_completion_only_after_native_finish() {
    let temp = tempfile::tempdir().unwrap();
    let (mut s, stage) = setup(temp.path());
    put(&stage, "opt/local/bin/app", "payload");
    s.invoke("proc", &["portdestroot::destroot_start", "args", ""])
        .unwrap();
    let item = s.scalar("org.macports.destroot").unwrap();
    assert_eq!(
        s.run_prepared_target(&item, false).unwrap(),
        TargetOutcome::Completed
    );
    assert_eq!(
        s.run_prepared_target(&item, false).unwrap(),
        TargetOutcome::AlreadyCompleted
    );
}
#[test]
fn missing_mask_or_metadata_session_cannot_begin_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let (mut s, stage) = setup(temp.path());
    put(&stage, "opt/local/share/info/dir", "preserve");
    s.invoke("unset", &["portdestroot::oldmask"]).unwrap();
    assert!(s.invoke("portdestroot::destroot_finish", &[]).is_err());
    assert!(stage.join("opt/local/share/info/dir").exists());
    let mut metadata = Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    assert!(
        metadata
            .invoke("portdestroot::destroot_finish", &[])
            .is_err()
    );
}
#[test]
fn sparse_workaround_is_best_effort_and_preserves_original_file() {
    let temp = tempfile::tempdir().unwrap();
    let (mut s, stage) = setup(temp.path());
    let work = temp.path().join("scratch");
    fs::create_dir(&work).unwrap();
    s.invoke("set", &["workpath", work.to_str().unwrap()])
        .unwrap();
    put(&stage, "opt/local/lib/sparse", "begin");
    let path = stage.join("opt/local/lib/sparse");
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(1024 * 1024)
        .unwrap();
    s.invoke("portdestroot::destroot_finish", &[]).unwrap();
    let bytes = fs::read(path).unwrap();
    assert_eq!(bytes.len(), 1024 * 1024);
    assert_eq!(&bytes[..5], b"begin");
    assert!(bytes[5..].iter().all(|byte| *byte == 0));
    assert_eq!(fs::read_dir(work).unwrap().count(), 0);
}
