use port_rs::{
    portfile::{self, Evaluation},
    spec::PortSpec,
};
use std::collections::BTreeMap;
fn run(script: &str, arch: &str) -> anyhow::Result<Evaluation> {
    let vars = BTreeMap::from([
        ("os.platform".into(), "darwin".into()),
        ("os.subplatform".into(), "macosx".into()),
        ("os.major".into(), "24".into()),
        ("os.arch".into(), arch.into()),
    ]);
    portfile::evaluate(
        &format!("name app; version 1\n{script}"),
        &vars,
        &PortSpec::named("app"),
    )
}
#[test]
fn matches_platform_subplatform_and_architecture_short_forms() {
    for arch in ["arm", "i386"] {
        let script = format!(
            "platform darwin {{ depends_lib-append port:darwin }}; platform macosx {{ depends_lib-append port:macosx }}; platform {arch} {{ depends_lib-append port:cpu }}"
        );
        assert_eq!(
            run(&script, arch).unwrap().port.get("depends_lib"),
            "port:darwin port:macosx port:cpu"
        );
    }
}
#[test]
fn matches_release_and_cpu_combinations_with_exact_os_comparison() {
    let script = r#"
        platform darwin 24 arm { depends_lib-append port:matching }
        platform darwin arm 24 { depends_lib-append port:reordered }
        platform macosx 24 { depends_lib-append port:subplatform }
        platform darwin 23 { error wrong-release }
        platform darwin 24 i386 { error wrong-cpu }
        platform freebsd { exec must-not-run }
        platform dar* { error no-globs }
    "#;
    assert_eq!(
        run(script, "arm").unwrap().port.get("depends_lib"),
        "port:matching port:reordered port:subplatform"
    );
}
#[test]
fn executes_in_callers_scope_and_can_declare_variants() {
    let script = r#"
        proc configure_platform {} {
            set selected no
            platform darwin { set selected yes; depends_lib-append port:local }
            if {$selected ne "yes"} { error wrong-scope }
        }
        configure_platform
        platform arm { variant native { depends_lib-append port:native }; default_variants +native }
    "#;
    let result = run(script, "arm").unwrap();
    assert_eq!(result.enabled, "+native");
    assert_eq!(result.port.get("depends_lib"), "port:local port:native");
}
#[test]
fn matching_errors_propagate_and_bad_arity_is_rejected() {
    assert!(run("platform darwin { error selected-failure }", "arm").is_err());
    assert!(run("platform darwin", "arm").is_err());
    assert!(run("platform", "arm").is_err());
    assert!(run("platform darwin { exec forbidden }", "arm").is_err());
}
#[test]
fn platform_properties_are_read_only_when_the_condition_needs_them() {
    let vars = BTreeMap::from([("os.platform".into(), "darwin".into())]);
    let spec = PortSpec::named("app");
    assert!(
        portfile::evaluate(
            "name app; version 1; platform darwin {revision 2}",
            &vars,
            &spec
        )
        .is_ok()
    );
    assert!(portfile::evaluate("name app; version 1; platform arm {}", &vars, &spec).is_err());
}
#[test]
fn later_release_and_arch_arguments_override_earlier_ones_like_upstream() {
    assert_eq!(
        run("platform darwin 23 i386 24 arm { revision 2 }", "arm")
            .unwrap()
            .port
            .version(),
        "1_2"
    );
}
