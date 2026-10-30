use port_rs::{
    build::{self, Phase},
    portfile::{PortGroups, Session},
    spec::PortSpec,
    tcl_list,
};
use std::{
    collections::BTreeMap,
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::{Arc, Mutex},
};
fn open(body: &str) -> Session {
    Session::open(
        &format!(
            r#"
        name app; version 1
        set os.platform darwin
        set prefix /opt/local
        set macosx_deployment_target {{}}
        set configure.sdkroot {{}}
        set configure.developer_dir {{}}
        set source_date_epoch {{}}
        set compiler.log_verbose_output no
        set compiler.cpath {{}}
        set compiler.library_path {{}}
        set system_options(clonebin_path) {{}}
        build.cmd /usr/bin/make
        option build.dir .
        {body}
    "#
        ),
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap()
}
fn enable(session: &mut Session, root: &Path) -> Arc<Mutex<Vec<u8>>> {
    let output = Arc::new(Mutex::new(Vec::new()));
    let captured = output.clone();
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
                let mut bytes = Vec::new();
                let result = runtime.block_on(build::run_status_to(
                    &command,
                    "app",
                    Phase::Destroot,
                    &mut bytes,
                ));
                captured.lock().unwrap().extend(bytes);
                result
            },
        )
        .unwrap();
    output
}
#[test]
fn stages_install_target_under_destdir_and_restores_clone_wrapper_path() {
    let tree = tempfile::tempdir().unwrap();
    let stage = tree.path().join("stage");
    let wrappers = tree.path().join("wrappers");
    std::fs::create_dir(&wrappers).unwrap();
    std::fs::write(
        wrappers.join("cp"),
        "#!/bin/sh\necho clone-wrapper\nexec /bin/cp \"$@\"\n",
    )
    .unwrap();
    std::fs::set_permissions(wrappers.join("cp"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(tree.path().join("hello"), "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(
        tree.path().join("hello"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    std::fs::write(tree.path().join("Makefile"),"install:\n\tmkdir -p \"$(DESTDIR)/opt/local/bin\"\n\tcp hello \"$(DESTDIR)/opt/local/bin/hello\"\n").unwrap();
    let mut session = open("destroot {do-destroot}");
    session
        .invoke("set", &["destpath", stage.to_str().unwrap()])
        .unwrap();
    session
        .invoke(
            "set",
            &["system_options(clonebin_path)", wrappers.to_str().unwrap()],
        )
        .unwrap();
    let output = enable(&mut session, tree.path());
    assert_eq!(session.invoke("do-destroot", &[]).unwrap(), "0");
    let installed = stage.join("opt/local/bin/hello");
    assert_eq!(
        std::fs::read_to_string(&installed).unwrap(),
        "#!/bin/sh\nexit 0\n"
    );
    assert_ne!(
        std::fs::metadata(installed).unwrap().permissions().mode() & 0o111,
        0
    );
    assert_eq!(
        session.invoke("set", &["env(PATH)"]).unwrap(),
        "/usr/bin:/bin"
    );
    let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    assert!(output.contains("-> staging app: clone-wrapper"));
    assert!(
        output
            .lines()
            .all(|line| line.starts_with("-> staging app: "))
    );
}
#[test]
fn staging_failure_preserves_child_status_and_restores_path_and_environment() {
    let tree = tempfile::tempdir().unwrap();
    let mut session = open(
        r#"
        destroot.cmd /bin/sh
        destroot.pre_args {}
        destroot.post_args {}
        destroot.args -c {'echo staging-failed; exit 9'}
        destroot.env LOCAL=temporary
        set system_options(clonebin_path) /wrapper
        proc failure {} {
            catch {portdestroot::destroot_main} message options
            return [dict get $options -errorcode]
        }
    "#,
    );
    enable(&mut session, tree.path());
    let code = tcl_list::parse(&session.invoke("failure", &[]).unwrap()).unwrap();
    assert_eq!(code[0], "CHILDSTATUS");
    assert_eq!(code[2], "9");
    assert_eq!(
        session.invoke("set", &["env(PATH)"]).unwrap(),
        "/usr/bin:/bin"
    );
    assert_eq!(
        session.invoke("info", &["exists", "env(LOCAL)"]).unwrap(),
        "0"
    );
}
#[test]
fn staging_arguments_follow_make_type_and_honor_portfile_overrides() {
    let mut session = open("");
    session
        .enable_system(&BTreeMap::new(), |_| {
            panic!("argument lookup must not execute")
        })
        .unwrap();
    session.invoke("set", &["destpath", "/stage"]).unwrap();
    assert_eq!(session.scalar("destroot.pre_args").unwrap(), "-w install");
    assert_eq!(
        session.scalar("destroot.post_args").unwrap(),
        "DESTDIR=/stage"
    );
    session.invoke("set", &["os.platform", "freebsd"]).unwrap();
    assert_eq!(session.scalar("destroot.pre_args").unwrap(), "install");
    session
        .invoke("set", &["destroot.target", "custom"])
        .unwrap();
    session
        .invoke("set", &["destroot.destdir", "STAGE=/custom"])
        .unwrap();
    assert_eq!(session.scalar("destroot.pre_args").unwrap(), "custom");
    assert_eq!(
        session.scalar("destroot.post_args").unwrap(),
        "STAGE=/custom"
    );
}
#[test]
fn metadata_and_uninitialized_clone_configuration_cannot_launch_staging() {
    assert!(
        Session::open(
            "name app; version 1; catch {portdestroot::destroot_main}",
            &BTreeMap::new(),
            &PortSpec::named("app"),
            PortGroups::default()
        )
        .is_err()
    );
    let mut session = open("unset system_options(clonebin_path)");
    session
        .enable_system(&BTreeMap::new(), |_| {
            panic!("uninitialized configuration must not launch")
        })
        .unwrap();
    assert!(
        session
            .invoke("portdestroot::destroot_main", &[])
            .unwrap_err()
            .to_string()
            .contains("clonebin_path")
    );
}
