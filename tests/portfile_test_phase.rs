use port_rs::{
    build::{self, Phase},
    portfile::{PortGroups, Session},
    spec::PortSpec,
    tcl_list,
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};
fn open(body: &str) -> Session {
    Session::open(
        &format!(
            r#"
        name app; version 1
        set os.platform darwin
        test.ignore_archs yes
        set macosx_deployment_target {{}}
        set configure.sdkroot {{}}
        set configure.developer_dir {{}}
        set source_date_epoch {{}}
        set compiler.log_verbose_output no
        set compiler.cpath {{}}
        set compiler.library_path {{}}
        option build.dir .
        build.cmd /usr/bin/make
        {body}
    "#
        ),
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap()
}
fn enable(session: &mut Session, directory: &Path) -> Arc<Mutex<Vec<u8>>> {
    let output = Arc::new(Mutex::new(Vec::new()));
    let captured = output.clone();
    let directory = directory.to_owned();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    session
        .enable_system(
            &BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
            move |request| {
                let command = request.into_command(&directory)?;
                let mut bytes = Vec::new();
                let result = runtime.block_on(build::run_status_to(
                    &command,
                    "app",
                    Phase::Test,
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
fn test_phase_runs_inherited_make_command_and_streams_testing_logs() {
    let tree = tempfile::tempdir().unwrap();
    std::fs::write(
        tree.path().join("Makefile"),
        "test:\n\t@echo test-output\n\t@echo test-warning >&2\n\t@touch passed\n",
    )
    .unwrap();
    let mut session = open("test.run yes\ntest {do-test}");
    let output = enable(&mut session, tree.path());
    assert_eq!(session.invoke("do-test", &[]).unwrap(), "0");
    assert!(tree.path().join("passed").exists());
    let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    assert!(output.contains("-> testing app: test-output\n"));
    assert!(output.contains("-> testing app: test-warning\n"));
}
#[test]
fn missing_or_false_test_run_skips_port_tests_but_not_architecture_checks() {
    for setting in ["", "test.run no"] {
        let mut session = open(setting);
        session
            .enable_system(&BTreeMap::new(), |_| {
                panic!("disabled tests must not execute")
            })
            .unwrap();
        assert_eq!(session.invoke("porttest::test_main", &[]).unwrap(), "0");
    }
    let mut session = open("test.run no\ntest.ignore_archs no");
    session
        .enable_system(&BTreeMap::new(), |_| {
            panic!("architecture check must stop first")
        })
        .unwrap();
    assert!(
        session
            .invoke("porttest::test_main", &[])
            .unwrap_err()
            .to_string()
            .contains("porttest::test_archs")
    );
}
#[test]
fn failed_test_command_retains_child_status_and_restores_environment() {
    let tree = tempfile::tempdir().unwrap();
    let mut session = open(
        r#"
        test.run yes
        test.cmd /bin/sh
        test.target {}
        test.args -c {'echo failed; exit 7'}
        test.env TEMP_TEST=value
        proc failed {} {
            catch {porttest::test_main} message options
            return [dict get $options -errorcode]
        }
    "#,
    );
    let output = enable(&mut session, tree.path());
    let code = tcl_list::parse(&session.invoke("failed", &[]).unwrap()).unwrap();
    assert_eq!(code[0], "CHILDSTATUS");
    assert_eq!(code[2], "7");
    assert_eq!(
        session
            .invoke("info", &["exists", "env(TEMP_TEST)"])
            .unwrap(),
        "0"
    );
    assert!(
        String::from_utf8(output.lock().unwrap().clone())
            .unwrap()
            .contains("-> testing app: failed")
    );
}
#[test]
fn metadata_cannot_run_tests_and_non_darwin_skips_architecture_checks() {
    assert!(
        Session::open(
            "name app; version 1; catch {porttest::test_main}",
            &BTreeMap::new(),
            &PortSpec::named("app"),
            PortGroups::default()
        )
        .is_err()
    );
    let mut session = open("set os.platform freebsd\ntest.ignore_archs no");
    session
        .enable_system(&BTreeMap::new(), |_| panic!("no test requested"))
        .unwrap();
    assert_eq!(session.invoke("porttest::test_main", &[]).unwrap(), "0");
}
