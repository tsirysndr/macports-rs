use port_rs::{
    build::state::StateFile,
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{collections::BTreeMap, fs};
#[test]
fn destroot_completion_requires_directory_and_state_and_preserves_borrowed_handle() {
    let root = tempfile::tempdir().unwrap();
    let source = "name app; version 1";
    fs::write(root.path().join("Portfile"), source).unwrap();
    let path = |p: &str| root.path().join(p).to_str().unwrap().to_owned();
    let mut s = Session::open(
        source,
        &BTreeMap::from([
            ("portpath".into(), path("")),
            ("portdbpath".into(), path("database")),
            ("portbuildpath".into(), path("build")),
            ("subbuildpath".into(), path("build/app")),
            ("workpath".into(), path("build/app/work")),
            ("destroot".into(), path("destroot")),
        ]),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    s.enable_system(&BTreeMap::new(), |_| panic!("no shell needed"))
        .unwrap();
    let check = "portarchivefetch::check_destroot_done";
    assert_eq!(s.invoke(check, &[]).unwrap(), "0");
    assert!(!root.path().join("build").exists());
    fs::create_dir(root.path().join("destroot")).unwrap();
    assert_eq!(s.invoke(check, &[]).unwrap(), "0");
    assert_eq!(
        s.invoke("info", &["exists", "target_state_fd"]).unwrap(),
        "0"
    );
    let state_path = root.path().join("build/app/work/.macports.app.state");
    let state = StateFile::try_open(&state_path, None)
        .unwrap()
        .expect("helper released lock");
    let fd = s.attach_build_state(state).unwrap();
    s.invoke("set", &["target_state_fd", &fd]).unwrap();
    s.invoke("write_statefile", &["target", "org.macports.destroot", &fd])
        .unwrap();
    assert_eq!(s.invoke(check, &[]).unwrap(), "1");
    assert_eq!(s.scalar("target_state_fd").unwrap(), fd);
    s.close_build_state(&fd).unwrap();
    s.invoke("unset", &["target_state_fd"]).unwrap();
    assert_eq!(s.invoke(check, &[]).unwrap(), "1");
    assert!(StateFile::try_open(&state_path, None).unwrap().is_some());
    s.invoke("set", &["target_state_fd", "invalid"]).unwrap();
    assert!(
        s.invoke(check, &[])
            .unwrap_err()
            .to_string()
            .contains("unknown build-state handle")
    );
    assert_eq!(s.scalar("target_state_fd").unwrap(), "invalid");
    fs::remove_dir(root.path().join("destroot")).unwrap();
    assert_eq!(s.invoke(check, &[]).unwrap(), "0");
}
