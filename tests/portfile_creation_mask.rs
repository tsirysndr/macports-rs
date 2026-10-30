use port_rs::{
    build::{self, Phase},
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{collections::BTreeMap, os::unix::fs::PermissionsExt, path::Path};
fn open() -> Session {
    Session::open(
        r#"
        name app; version 1
        set macosx_deployment_target {}
        set configure.sdkroot {}
        set configure.developer_dir {}
        set source_date_epoch {}
        set compiler.log_verbose_output no
        set compiler.cpath {}
        set compiler.library_path {}
        option build.dir nested/deep
        build.cmd /bin/sh
        build.args -c {': > artifact'}
    "#,
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap()
}
fn enable(session: &mut Session, root: &Path) {
    let root = root.to_owned();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    session
        .enable_system(
            &BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
            move |request| {
                let command = request.into_command(&root)?;
                runtime.block_on(build::run_status_to(
                    &command,
                    "app",
                    Phase::Destroot,
                    &mut Vec::new(),
                ))
            },
        )
        .unwrap();
}
fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}
#[test]
fn session_masks_control_children_and_prepared_directories_without_changing_host() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(root.path().join("before"), "").unwrap();
    let mut session = open();
    enable(&mut session, root.path());
    session.initialize_creation_mask(0o022).unwrap();
    assert_eq!(session.invoke("umask", &["077"]).unwrap(), "0022");
    session.invoke("command_exec", &["build"]).unwrap();
    assert_eq!(mode(&root.path().join("nested")), 0o700);
    assert_eq!(mode(&root.path().join("nested/deep")), 0o700);
    assert_eq!(mode(&root.path().join("nested/deep/artifact")), 0o600);
    assert_eq!(mode(root.path()), 0o755);
    assert_eq!(session.invoke("umask", &["000"]).unwrap(), "0077");
    session.invoke("set", &["build.dir", "open/deep"]).unwrap();
    session.invoke("command_exec", &["build"]).unwrap();
    assert_eq!(mode(&root.path().join("open/deep")), 0o777);
    assert_eq!(mode(&root.path().join("open/deep/artifact")), 0o666);
    session
        .invoke("system", &[": > permissive; mkdir permissive-dir"])
        .unwrap();
    assert_eq!(mode(&root.path().join("permissive")), 0o666);
    assert_eq!(mode(&root.path().join("permissive-dir")), 0o777);
    std::fs::write(root.path().join("after"), "").unwrap();
    assert_eq!(
        mode(&root.path().join("before")),
        mode(&root.path().join("after"))
    );
}
#[test]
fn old_masks_symbolic_modes_invalid_modes_and_initialization_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    let mut session = open();
    enable(&mut session, root.path());
    assert!(
        session
            .invoke("umask", &["022"])
            .unwrap_err()
            .to_string()
            .contains("not initialized")
    );
    session.initialize_creation_mask(0o022).unwrap();
    assert_eq!(session.invoke("umask", &["u=rw,g=r,o=r"]).unwrap(), "0022");
    assert!(
        session
            .invoke("umask", &["invalid"])
            .unwrap_err()
            .to_string()
            .contains("Invalid umask mode")
    );
    assert!(session.initialize_creation_mask(0o1000).is_err());
    assert_eq!(session.invoke("umask", &["022"]).unwrap(), "0644");
    assert!(session.invoke("umask", &[]).is_err());
    assert_eq!(session.invoke("umask", &["022"]).unwrap(), "0022");
}
#[test]
fn metadata_cannot_change_the_creation_mask() {
    assert!(
        Session::open(
            "name app; version 1; catch {umask 077}",
            &BTreeMap::new(),
            &PortSpec::named("app"),
            PortGroups::default()
        )
        .is_err()
    );
}
