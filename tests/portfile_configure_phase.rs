use port_rs::{
    build::{self, Phase},
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};
type Requests = Arc<Mutex<Vec<(String, BTreeMap<String, String>)>>>;
type Output = Arc<Mutex<Vec<u8>>>;
fn open(body: &str) -> Session {
    Session::open(&format!(r#"
        name app; version 1
        set os.platform darwin
        set os.major 12
        set prefix /configured
        set macosx_deployment_target {{}}
        set configure.sdkroot {{}}
        set source_date_epoch 100
        set compiler.log_verbose_output no
        set compiler.cpath {{}}
        set compiler.library_path {{}}
        set compiler.limit_flags no
        foreach config_field {{cc cxx objc objcxx fc f77 f90 javac cflags cppflags cxxflags objcflags objcxxflags fflags f90flags fcflags ldflags libs classpath perl python ruby install awk bison pkg_config developer_dir pkg_config_path cc_archflags}} {{
            set configure.$config_field {{}}
        }}
        configure.cmd /bin/sh
        configure.args configure-script
        option configure.dir .
        option build.dir .
        {body}
    "#), &BTreeMap::new(), &PortSpec::named("app"), PortGroups::default()).unwrap()
}
fn enable(session: &mut Session, root: &Path) -> (Requests, Output) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let saved = requests.clone();
    let output = Arc::new(Mutex::new(Vec::new()));
    let bytes = output.clone();
    let root = root.to_owned();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    session
        .enable_system(
            &BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
            move |request| {
                saved
                    .lock()
                    .unwrap()
                    .push((request.script.clone(), request.environment.clone()));
                let command = request.into_command(&root)?;
                let mut out = Vec::new();
                let status = runtime.block_on(build::run_status_to(
                    &command,
                    "app",
                    Phase::Configure,
                    &mut out,
                ));
                bytes.lock().unwrap().extend(out);
                status
            },
        )
        .unwrap();
    (requests, output)
}
#[test]
fn configure_executes_with_compiler_sdk_architecture_and_user_environment() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("configure-script"),
        "echo configure-ready\nprintf 'all:\\n\\t@echo built\\n' > Makefile\n",
    )
    .unwrap();
    let mut session = open(
        r#"
        configure.pre_args {}
        configure.env CC=wrapper CFLAGS=initial LOCAL=temporary
        configure.cc clang
        configure.cflags -O2
        configure.libs -lm
        configure.pkg_config_path /one {/two words}
        configure.cc_archflags -arch arm64
        configure.ccache yes
        configure.distcc yes
        configure.pipe yes
        configure.march cpu
        configure.mtune tune
        option configure.sdkroot {/sdk path}
        configure {do-configure}
    "#,
    );
    let (requests, output) = enable(&mut session, root.path());
    assert_eq!(session.invoke("do-configure", &[]).unwrap(), "0");
    let requests = requests.lock().unwrap();
    let env = &requests[0].1;
    assert_eq!(env["CC"], "wrapper ccache clang");
    assert_eq!(env["CCACHE_PREFIX"], "distcc");
    assert_eq!(
        env["CFLAGS"],
        "initial -pipe -O2 -isysroot/sdk path -arch arm64 -march=cpu -mtune=tune"
    );
    assert_eq!(
        env["LDFLAGS"],
        "-Wl,-syslibroot,/sdk path -march=cpu -mtune=tune"
    );
    assert_eq!(env["PKG_CONFIG_PATH"], "/one:/two words");
    assert_eq!(env["__CFPREFERENCES_AVOID_DAEMON"], "1");
    assert_eq!(env["LIBS"], "-lm");
    assert!(root.path().join("Makefile").is_file());
    assert!(
        String::from_utf8(output.lock().unwrap().clone())
            .unwrap()
            .contains("-> configuring app: configure-ready")
    );
    assert_eq!(
        session.invoke("info", &["exists", "env(LOCAL)"]).unwrap(),
        "0"
    );
}
#[test]
fn autotools_run_in_order_and_failure_prevents_later_commands() {
    let root = tempfile::tempdir().unwrap();
    for tool in ["autoreconf", "automake", "autoconf"] {
        std::fs::write(root.path().join(tool), format!("echo {tool} >> order\n")).unwrap();
    }
    let mut session = open(
        r#"
        use_configure no
        use_autoreconf yes
        use_automake yes
        use_autoconf yes
        foreach tool {autoreconf automake autoconf} {
            option ${tool}.dir .
            option ${tool}.cmd /bin/sh
            option ${tool}.args $tool
        }
    "#,
    );
    let (requests, _) = enable(&mut session, root.path());
    assert_eq!(
        session
            .invoke("portconfigure::configure_main", &[])
            .unwrap(),
        "0"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("order")).unwrap(),
        "autoreconf\nautomake\nautoconf\n"
    );
    std::fs::write(root.path().join("autoreconf"), "exit 7\n").unwrap();
    let failure = session
        .invoke("portconfigure::configure_main", &[])
        .unwrap_err();
    assert!(failure.to_string().contains("autoreconf failure"));
    assert_eq!(requests.lock().unwrap().len(), 4);
}
#[test]
fn universal_flags_replace_single_arch_flags_and_append_configure_arguments() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("configure-script"), "echo configured\n").unwrap();
    let mut session = open(
        r#"
        variant universal {}
        default_variants +universal
        set configure.universal_archs {arm64 x86_64}
        configure.cc_archflags -arch arm64
        configure.universal_cflags -arch arm64 -arch x86_64
        configure.march must-not-appear
        configure.cmd /bin/sh
        configure.pre_args configure-script
        configure.args {}
    "#,
    );
    let (requests, _) = enable(&mut session, root.path());
    session
        .invoke("portconfigure::configure_main", &[])
        .unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests[0].1["CFLAGS"], "-pipe -arch arm64 -arch x86_64");
    assert!(
        requests[0]
            .0
            .contains("configure-script --disable-dependency-tracking")
    );
}
#[test]
fn configure_failure_restores_environment_and_missing_inputs_do_not_launch() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("configure-script"), "exit 5\n").unwrap();
    let mut session = open("configure.pre_args {}\nconfigure.env LOCAL=temp");
    let (requests, _) = enable(&mut session, root.path());
    assert!(
        session
            .invoke("portconfigure::configure_main", &[])
            .unwrap_err()
            .to_string()
            .contains("configure failure")
    );
    assert_eq!(
        session.invoke("info", &["exists", "env(LOCAL)"]).unwrap(),
        "0"
    );
    session.invoke("unset", &["configure.cc"]).unwrap();
    assert!(
        session
            .invoke("portconfigure::configure_main", &[])
            .unwrap_err()
            .to_string()
            .contains("configure.cc")
    );
    assert_eq!(requests.lock().unwrap().len(), 1);
}
#[test]
fn metadata_cannot_run_configure_and_disabled_configuration_is_a_noop() {
    assert!(
        Session::open(
            "name app; version 1; catch {portconfigure::configure_main}",
            &BTreeMap::new(),
            &PortSpec::named("app"),
            PortGroups::default()
        )
        .is_err()
    );
    let mut session = open("use_configure no");
    session
        .enable_system(&BTreeMap::new(), |_| {
            panic!("disabled configure must not execute")
        })
        .unwrap();
    assert_eq!(
        session
            .invoke("portconfigure::configure_main", &[])
            .unwrap(),
        "0"
    );
}

#[test]
fn xmkmf_runs_makefiles_generation_instead_of_the_configure_command() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("xmkmf-script"),
        "printf 'Makefiles:\\n\\t@touch generated\\n' > Makefile\n",
    )
    .unwrap();
    let mut session = open(
        r#"
        use_xmkmf yes
        use_configure yes
        xmkmf.cmd /bin/sh
        xmkmf.args xmkmf-script
        option xmkmf.dir .
        unset configure.cc
    "#,
    );
    session
        .invoke("set", &["worksrcpath", root.path().to_str().unwrap()])
        .unwrap();
    let (requests, _) = enable(&mut session, root.path());
    assert_eq!(
        session
            .invoke("portconfigure::configure_main", &[])
            .unwrap(),
        "0"
    );
    assert!(root.path().join("generated").exists());
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].0.contains("xmkmf-script"));
    assert!(requests[1].0.contains("make Makefiles"));
}

#[test]
fn prepared_configure_runs_start_main_finish_and_skips_completed_work() {
    use port_rs::{portfile::TargetOutcome, progress::Progress};
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    std::fs::write(root.join("Portfile"), "name app; version 1").unwrap();
    std::fs::write(root.join("configure-script"), "echo configure-lifecycle\nprintf 'all:\\n\\t@echo built\\n' > Makefile\necho 'warning: type defaults to int [-Wimplicit-int]' > config.log\n").unwrap();
    let mut session = open(
        "configure.compiler clang; configure.pre_args {}; configure.checks.implicit_function_declaration.whitelist {}",
    );
    let work = root.join("build/new/app/work");
    for (key, value) in [
        ("portpath", root.clone()),
        ("portdbpath", root.join("database")),
        ("portbuildpath", root.join("build/new")),
        ("subbuildpath", root.join("build/new/app")),
        ("workpath", work.clone()),
        ("configure.dir", work.join("src")),
    ] {
        session
            .invoke("set", &[key, value.to_str().unwrap()])
            .unwrap();
    }
    session
        .invoke(
            "configure.args",
            &[root.join("configure-script").to_str().unwrap()],
        )
        .unwrap();
    let (requests, output) = enable(&mut session, &root);
    session.set_progress(Progress::new(true));
    let item = session.scalar("org.macports.configure").unwrap();
    assert_eq!(
        session.run_prepared_target(&item, false).unwrap(),
        TargetOutcome::Completed
    );
    assert!(work.join("src/Makefile").is_file());
    assert!(
        String::from_utf8(output.lock().unwrap().clone())
            .unwrap()
            .contains("-> configuring app: configure-lifecycle")
    );
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(
        session.run_prepared_target(&item, false).unwrap(),
        TargetOutcome::AlreadyCompleted
    );
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[test]
fn configure_start_rejects_unknown_compiler_before_running_any_commands() {
    let mut session = open("configure.compiler unknown");
    session
        .enable_system(&BTreeMap::new(), |_| {
            panic!("startup must not launch a command")
        })
        .unwrap();
    session.set_progress(port_rs::progress::Progress::new(true));
    assert!(
        session
            .invoke("portconfigure::configure_start", &[])
            .unwrap_err()
            .to_string()
            .contains("Invalid value for configure.compiler: unknown")
    );
    session
        .invoke("configure.compiler", &["macports-clang-devel"])
        .unwrap();
    assert_eq!(
        session
            .invoke("portconfigure::configure_start", &["ignored"])
            .unwrap(),
        ""
    );
    session.invoke("configure.ccache", &["yes"]).unwrap();
    let error = session
        .invoke("portconfigure::configure_start", &[])
        .unwrap_err()
        .to_string();
    if unsafe { libc::getuid() } == 0 {
        assert!(error.contains("privilege"));
    } else {
        assert!(error.contains("requires root privileges"));
    }
}
