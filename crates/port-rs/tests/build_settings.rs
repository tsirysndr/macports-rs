use port_rs::{
    portfile,
    source::{BuildHost, Platform},
    spec::PortSpec,
};

fn evaluate(script: &str, variables: &[(&str, &str)], modifiers: &[&str]) -> portfile::Evaluation {
    let mut spec = PortSpec::named("app");
    for modifier in modifiers {
        spec.parse_modifier(modifier).unwrap();
    }
    let variables = variables
        .iter()
        .map(|(k, v)| ((*k).into(), (*v).into()))
        .collect();
    portfile::evaluate(&format!("name app; version 1; {script}"), &variables, &spec).unwrap()
}

#[test]
fn job_defaults_follow_cpu_memory_parallel_flags_and_explicit_settings() {
    let host = [
        ("port_rs.active_cpus", "16"),
        ("port_rs.memory_bytes", "8589934592"),
    ];
    assert_eq!(evaluate("", &host, &[]).port.get("build.jobs"), "9");
    assert_eq!(
        evaluate("build.mem_per_job 2048", &host, &[])
            .port
            .get("build.jobs"),
        "5"
    );
    assert_eq!(
        evaluate("use_parallel_build no", &host, &[])
            .port
            .get("build.jobs"),
        "1"
    );
    assert_eq!(
        evaluate("set buildmakejobs 12", &host, &[])
            .port
            .get("build.jobs"),
        "12"
    );
    assert_eq!(
        evaluate("set buildmakejobs -4", &host, &[])
            .port
            .get("build.jobs"),
        "1"
    );
    assert_eq!(
        evaluate("set buildmakejobs invalid", &host, &[])
            .port
            .get("build.jobs"),
        "1"
    );
    assert_eq!(
        evaluate("build.mem_per_job 0", &host, &[])
            .port
            .get("build.jobs"),
        "16"
    );
    assert_eq!(evaluate("", &[], &[]).port.get("build.jobs"), "2");
    assert_eq!(
        evaluate(
            "use_parallel_build no; build.jobs 1",
            &host,
            &["build.jobs=7"]
        )
        .port
        .get("build.jobs"),
        "7"
    );
}

#[test]
fn job_defaults_are_lazy_and_do_not_capture_old_option_values() {
    let value = evaluate(
        r#"
        set before ${build.jobs}
        build.mem_per_job 4096
        set middle ${build.jobs}
        use_parallel_build no
        description {*}"$before $middle ${build.jobs}"
    "#,
        &[
            ("port_rs.active_cpus", "16"),
            ("port_rs.memory_bytes", "8589934592"),
        ],
        &[],
    );
    assert_eq!(value.port.get("description"), "9 3 1");
}

#[test]
fn sdk_and_supported_architectures_filter_and_demote_in_native_order() {
    let host = [
        ("build_arch", "arm64"),
        ("universal_archs", "arm64 x86_64"),
        ("macosx_sdk_version", "15"),
    ];
    let value = evaluate("", &host, &[]);
    assert_eq!(value.port.get("configure.build_arch"), "arm64");
    assert_eq!(value.port.get("configure.universal_archs"), "arm64 x86_64");
    let value = evaluate("supported_archs x86_64", &host, &[]);
    assert_eq!(value.port.get("configure.build_arch"), "x86_64");
    assert_eq!(value.port.get("configure.universal_archs"), "x86_64");
    assert_eq!(
        evaluate("supported_archs i386", &host, &[])
            .port
            .get("configure.universal_archs"),
        ""
    );
    let value = evaluate(
        "configure.sdk_version 10.13; supported_archs i386",
        &host,
        &[],
    );
    assert_eq!(value.port.get("configure.universal_archs"), "i386");
    assert_eq!(
        evaluate("supported_archs noarch", &host, &[])
            .port
            .get("configure.build_arch"),
        ""
    );
    assert_eq!(
        evaluate(
            "supported_archs x86_64",
            &host,
            &["configure.universal_archs=arm64"]
        )
        .port
        .get("configure.universal_archs"),
        "arm64"
    );
}

#[test]
fn pure_library_evaluation_does_not_guess_a_host_architecture() {
    let value = evaluate("", &[], &[]);
    assert_eq!(value.port.get("configure.build_arch"), "");
    assert_eq!(value.port.get("configure.universal_archs"), "");
}

#[test]
fn host_defaults_use_product_version_including_macos_26() {
    for (darwin, product, deployment, sdk, universal) in [
        (25, "26.1", "26.0", "26", "arm64 x86_64"),
        (24, "15.6.1", "15.0", "15", "arm64 x86_64"),
        (19, "10.15.7", "10.15", "10.15", "x86_64"),
        (18, "10.14.6", "10.14", "10.14", "x86_64 i386"),
    ] {
        let host = BuildHost {
            platform: Platform {
                darwin_major: darwin,
                architecture: "i386",
            },
            macos_version: product.into(),
            active_cpus: 8,
            memory_bytes: Some(8 * 1024 * 1024 * 1024),
        };
        let vars = host.variables().unwrap();
        assert_eq!(vars["macosx_deployment_target"], deployment);
        assert_eq!(vars["macosx_sdk_version"], sdk);
        assert_eq!(vars["universal_archs"], universal);
        assert_eq!(vars["build_arch"], "x86_64");
    }
    let host = BuildHost {
        platform: Platform {
            darwin_major: 24,
            architecture: "arm",
        },
        macos_version: "15.0".into(),
        active_cpus: 8,
        memory_bytes: None,
    };
    assert_eq!(host.variables().unwrap()["build_arch"], "arm64");
    for product in ["", "garbage", "10", "9.9", "15.invalid"] {
        let bad = BuildHost {
            macos_version: product.into(),
            platform: Platform {
                darwin_major: 24,
                architecture: "arm",
            },
            active_cpus: 8,
            memory_bytes: None,
        };
        assert!(bad.variables().is_err(), "{product}");
    }
}

#[test]
fn native_host_detection_supplies_valid_explicit_inputs() {
    let vars = port_rs::source::detect_build_host()
        .unwrap()
        .variables()
        .unwrap();
    assert!(vars["port_rs.active_cpus"].parse::<usize>().unwrap() > 0);
    assert!(matches!(vars["build_arch"].as_str(), "arm64" | "x86_64"));
    assert!(!vars["macosx_deployment_target"].is_empty());
    let value = portfile::evaluate("name app; version 1", &vars, &PortSpec::named("app")).unwrap();
    assert!(value.port.get("build.jobs").parse::<usize>().unwrap() > 0);
}
