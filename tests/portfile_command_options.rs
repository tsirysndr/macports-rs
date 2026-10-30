use port_rs::{portfile, spec::PortSpec, tcl_list};
use std::collections::BTreeMap;

#[test]
fn command_families_create_metadata_setters_without_executing_commands() {
    let value = portfile::evaluate(
        r#"
        name app; version 1
        commands custom
        use_custom yes
        custom.cmd {not-an-executable}
        custom.dir {/path with spaces}
        custom.pre_args before
        custom.args one {two words}
        custom.args-append three
        custom.args-prepend zero
        custom.args-delete one
        custom.args-replace three final
        custom.post_args after
        custom.env KEY=value
        custom.nice 5
        custom.type default
        commands custom
    "#,
        &BTreeMap::new(),
        &PortSpec::named("app"),
    )
    .unwrap();
    assert_eq!(
        tcl_list::parse(value.port.get("custom.args")).unwrap(),
        ["zero", "two words", "final"]
    );
    assert_eq!(value.port.get("custom.cmd"), "not-an-executable");
    assert_eq!(value.port.get("use_custom"), "yes");
    assert_eq!(
        tcl_list::parse(value.port.get("custom.dir")).unwrap(),
        ["/path with spaces"]
    );
    assert_eq!(value.port.get("custom.env"), "KEY=value");
}

#[test]
fn built_in_command_families_and_configure_fields_accept_overrides() {
    let mut spec = PortSpec::named("app");
    spec.parse_modifier("configure.cxxflags=-O3").unwrap();
    let value = portfile::evaluate(
        r#"
        name app; version 1
        configure.cxxflags -O0
        configure.cxxflags-append -g
        configure.checks.implicit_function_declaration.whitelist-append strchr
        configure.checks.implicit_int no
        configure.universal_objcxxflags -arch arm64
        compiler.cxx_standard 2020
        autoreconf.cmd autoreconf
        autoreconf.args -fvi
        build.cmd ninja
        destroot.pre_args install
        livecheck.type regex
        livecheck.regex {app-([0-9.]+)}
    "#,
        &BTreeMap::new(),
        &spec,
    )
    .unwrap();
    assert_eq!(value.port.get("configure.cxxflags"), "-O3");
    assert_eq!(
        value
            .port
            .get("configure.checks.implicit_function_declaration.whitelist"),
        "strchr"
    );
    assert_eq!(value.port.get("compiler.cxx_standard"), "2020");
    assert_eq!(value.port.get("build.cmd"), "ninja");
    assert_eq!(value.port.get("autoreconf.args"), "-fvi");
    assert_eq!(value.port.get("livecheck.type"), "regex");
}

#[test]
fn dynamically_declared_families_honor_requested_options_and_lazy_defaults() {
    let mut spec = PortSpec::named("app");
    spec.parse_modifier("custom.args=user").unwrap();
    let value = portfile::evaluate(
        r#"
        name app; version 1
        commands custom
        default custom.cmd {tool-${version}}
        custom.args default
        custom.args-append ignored
        version 2
    "#,
        &BTreeMap::new(),
        &spec,
    )
    .unwrap();
    assert_eq!(value.port.get("custom.args"), "user");
    assert_eq!(value.port.get("custom.cmd"), "tool-2");
}

#[test]
fn declarations_reject_invalid_names_and_existing_procedure_collisions() {
    for script in [
        "commands {}",
        "commands {bad;name}",
        "proc custom.cmd {} {}; commands custom",
    ] {
        assert!(
            portfile::evaluate(
                &format!("name app; version 1; {script}"),
                &BTreeMap::new(),
                &PortSpec::named("app")
            )
            .is_err(),
            "{script}"
        );
    }
}

#[test]
fn livecheck_url_can_be_derived_from_a_tagged_master_site() {
    let value = portfile::evaluate(
        r#"
        name app; version 1
        master_sites https://example.org/download/:release
        livecheck.type regex
        livecheck.url [join [lrange [split [lindex ${master_sites} 0] {:}] 0 end-1] {:}]
    "#,
        &BTreeMap::new(),
        &PortSpec::named("app"),
    )
    .unwrap();
    assert_eq!(
        value.port.get("livecheck.url"),
        "https://example.org/download/"
    );
}
