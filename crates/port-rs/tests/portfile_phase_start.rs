use port_rs::{
    build::{self, Phase},
    portfile::{PortGroups, Session, TargetOutcome},
    progress::Progress,
    spec::PortSpec,
};
use std::{
    collections::BTreeMap,
    fs,
    sync::{Arc, Mutex},
};

#[test]
fn built_in_build_and_test_run_with_startup_callbacks_and_resume_from_state() {
    for (target, phase, marker) in [
        ("build", Phase::Build, "built"),
        ("test", Phase::Test, "tested"),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        let project = root.join("project");
        fs::create_dir(&project).unwrap();
        fs::write(project.join("Makefile"), "all:\n\t@echo native-build\n\t@touch built\ntest:\n\t@echo native-test\n\t@touch tested\n").unwrap();
        let source = r#"
            name app; version 1
            set os.platform darwin
            set macosx_deployment_target {}
            set configure.sdkroot {}
            set configure.developer_dir {}
            set source_date_epoch {}
            set compiler.log_verbose_output no
            set compiler.cpath {}
            set compiler.library_path {}
            build.cmd /usr/bin/make
            build.jobs 1
            test.ignore_archs yes
            test.run yes
        "#;
        fs::write(root.join("Portfile"), source).unwrap();
        let vars = BTreeMap::from([
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
            ("build.dir".into(), project.to_str().unwrap().into()),
        ]);
        let mut s = Session::open(
            source,
            &vars,
            &PortSpec::named("app"),
            PortGroups::default(),
        )
        .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let output = Arc::new(Mutex::new(Vec::new()));
        let logs = output.clone();
        s.enable_system(
            &BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
            move |request| {
                let command = request.into_command(&project)?;
                let mut bytes = Vec::new();
                let result =
                    runtime.block_on(build::run_status_to(&command, "app", phase, &mut bytes));
                logs.lock().unwrap().extend(bytes);
                result
            },
        )
        .unwrap();
        s.set_progress(Progress::new(true));
        let item = s.scalar(&format!("org.macports.{target}")).unwrap();
        assert_eq!(
            s.run_prepared_target(&item, false).unwrap(),
            TargetOutcome::Completed
        );
        assert!(root.join("project").join(marker).exists());
        let first = output.lock().unwrap().clone();
        assert!(
            String::from_utf8(first.clone())
                .unwrap()
                .contains(&format!("native-{target}"))
        );
        assert_eq!(
            s.run_prepared_target(&item, false).unwrap(),
            TargetOutcome::AlreadyCompleted
        );
        assert_eq!(*output.lock().unwrap(), first);
    }
}

#[test]
fn metadata_cannot_run_phase_start_callbacks() {
    for command in ["portbuild::build_start", "porttest::test_start"] {
        let mut s = Session::open(
            "name app; version 1",
            &BTreeMap::new(),
            &PortSpec::named("app"),
            PortGroups::default(),
        )
        .unwrap();
        assert!(s.invoke(command, &[]).is_err());
    }
}
