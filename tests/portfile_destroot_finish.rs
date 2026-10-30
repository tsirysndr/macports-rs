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
    let share = root.join("share/install");
    fs::create_dir_all(&share).unwrap();
    for name in ["prefix.mtree", "macosx.mtree"] {
        fs::write(share.join(name), "/set type=dir mode=0755\n.\n..\n").unwrap();
    }
    let source = "name app; version 1; destroot {}";
    fs::write(root.join("Portfile"), source).unwrap();
    let vars = BTreeMap::from([
        ("destroot".into(), stage.to_str().unwrap().into()),
        ("prefix".into(), "/opt/local".into()),
        (
            "portsharepath".into(),
            root.join("share").to_str().unwrap().into(),
        ),
        ("os.platform".into(), "darwin".into()),
        ("destroot.umask".into(), "0022".into()),
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
    s.invoke("umask", &["0027"]).unwrap();
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
    fs::write(temp.path().join("payload"), "payload").unwrap();
    s.invoke("destroot", &["file mkdir ${destroot}${prefix}/bin; file rename [file join $portpath payload] ${destroot}${prefix}/bin/app"]).unwrap();
    let item = s.scalar("org.macports.destroot").unwrap();
    assert_eq!(
        s.run_prepared_target(&item, false).unwrap(),
        TargetOutcome::Completed
    );
    assert_eq!(
        s.run_prepared_target(&item, false).unwrap(),
        TargetOutcome::AlreadyCompleted
    );
    assert_eq!(
        fs::read_to_string(stage.join("opt/local/bin/app")).unwrap(),
        "payload"
    );
    assert_eq!(s.invoke("umask", &["0077"]).unwrap(), "0077");
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

#[test]
fn native_start_cleans_only_when_requested_and_saves_the_mask() {
    let temp = tempfile::tempdir().unwrap();
    let (mut s, stage) = setup(temp.path());
    put(&stage, "obsolete/file", "old");
    s.invoke("destroot.clean", &["yes"]).unwrap();
    s.invoke("portdestroot::destroot_start", &[]).unwrap();
    assert!(!stage.join("obsolete").exists());
    assert!(stage.join("opt/local").is_dir());
    assert!(stage.join("Applications/MacPorts").is_dir());
    assert_eq!(s.scalar("portdestroot::oldmask").unwrap(), "0077");
    assert_eq!(s.invoke("umask", &["0022"]).unwrap(), "0022");
}

#[test]
fn startup_failure_restores_mask_and_invalid_services_fail_before_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let (mut s, stage) = setup(temp.path());
    fs::remove_file(temp.path().join("share/install/prefix.mtree")).unwrap();
    assert!(s.invoke("portdestroot::destroot_start", &[]).is_err());
    assert_eq!(s.invoke("umask", &["0077"]).unwrap(), "0077");
    put(&stage, "preserve", "payload");
    s.invoke("startupitem.type", &["launchd"]).unwrap();
    s.invoke(
        "startupitem.custom_file",
        &["/missing/port-rs-service.plist"],
    )
    .unwrap();
    s.invoke("destroot.clean", &["yes"]).unwrap();
    let error = s.invoke("portdestroot::destroot_start", &[]).unwrap_err();
    assert!(error.to_string().contains("cannot read custom plist"));
    assert_eq!(
        fs::read_to_string(stage.join("preserve")).unwrap(),
        "payload"
    );
}

#[test]
fn failing_pre_destroot_hook_restores_mask_without_recording_completion() {
    let temp = tempfile::tempdir().unwrap();
    let (mut s, _) = setup(temp.path());
    s.invoke("pre-destroot", &["error {deliberate staging hook failure}"])
        .unwrap();
    let item = s.scalar("org.macports.destroot").unwrap();
    assert!(
        s.run_prepared_target(&item, false)
            .unwrap_err()
            .to_string()
            .contains("deliberate staging hook failure")
    );
    assert_eq!(s.invoke("umask", &["0077"]).unwrap(), "0077");
    let mut state = StateFile::open(
        &temp.path().join("build/new/app/work/.macports.app.state"),
        None,
    )
    .unwrap();
    assert!(!state.contains("target", "org.macports.destroot").unwrap());
}

#[test]
fn custom_startup_plist_passes_the_complete_native_staging_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    let (mut s, stage) = setup(temp.path());
    let input = temp.path().join("input.plist");
    fs::write(&input, "custom service").unwrap();
    s.invoke("startupitem.type", &["launchd"]).unwrap();
    s.invoke("startupitem.custom_file", &[input.to_str().unwrap()])
        .unwrap();
    let item = s.scalar("org.macports.destroot").unwrap();
    assert_eq!(
        s.run_prepared_target(&item, false).unwrap(),
        TargetOutcome::Completed
    );
    assert_eq!(
        fs::read_to_string(
            stage.join("opt/local/etc/LaunchDaemons/org.macports.app/org.macports.app.plist")
        )
        .unwrap(),
        "custom service"
    );
    assert_eq!(s.invoke("umask", &["0077"]).unwrap(), "0077");
}

#[test]
fn generated_launchd_plist_and_pid_directory_survive_native_finish() {
    let temp = tempfile::tempdir().unwrap();
    let (mut s, stage) = setup(temp.path());
    s.invoke("startupitem.type", &["launchd"]).unwrap();
    s.invoke("startupitem.create", &["yes"]).unwrap();
    s.invoke("startupitem.executable", &["/opt/local/bin/app"])
        .unwrap();
    s.invoke("startupitem.pidfile", &["auto"]).unwrap();
    s.invoke("startupitem.start", &["echo startup"]).unwrap();
    let item = s.scalar("org.macports.destroot").unwrap();
    assert_eq!(
        s.run_prepared_target(&item, false).unwrap(),
        TargetOutcome::Completed
    );
    assert!(
        stage
            .join("opt/local/etc/LaunchDaemons/org.macports.app/org.macports.app.plist")
            .is_file()
    );
    assert!(stage.join("opt/local/var/run/.turd_app").is_file());
    assert!(
        stage
            .join("opt/local/etc/LaunchDaemons/org.macports.app/app.wrapper")
            .is_file()
    );
    assert_eq!(s.invoke("umask", &["0077"]).unwrap(), "0077");
}
