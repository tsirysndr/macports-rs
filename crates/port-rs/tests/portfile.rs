use port_rs::{
    portfile::{self, Evaluation},
    spec::PortSpec,
    tcl_list,
};
use std::collections::BTreeMap;
fn environment() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("os.platform".into(), "darwin".into()),
        ("os.major".into(), "24".into()),
        ("prefix".into(), "/opt/local".into()),
    ])
}
fn evaluate(script: &str, modifiers: &[&str]) -> anyhow::Result<Evaluation> {
    let mut spec = PortSpec::named("app");
    for modifier in modifiers {
        spec.parse_modifier(modifier)?;
    }
    portfile::evaluate(script, &environment(), &spec)
}
#[test]
fn evaluates_substitution_expressions_procedures_loops_and_option_operations() {
    let result = evaluate(
        r#"
        PortSystem 1.0
        name app
        set major 2
        version ${major}.0
        revision [expr {1 + 2}]
        description {*}{A useful application}
        depends_lib port:first port:second port:first
        proc add_dependencies {names} {
            foreach name $names { depends_lib-append port:$name }
        }
        add_dependencies {third fourth}
        depends_lib-delete port:first
        depends_lib-prepend port:base
        depends_lib-replace port:second port:replacement
        if {${os.platform} eq "darwin" && ${os.major} >= 20} {
            configure.args --prefix=${prefix} \
                --with-system-library
        }
    "#,
        &[],
    )
    .unwrap();
    assert_eq!(result.port.version(), "2.0_3");
    assert_eq!(result.port.get("description"), "A useful application");
    assert_eq!(
        tcl_list::parse(result.port.get("depends_lib")).unwrap(),
        [
            "port:base",
            "port:replacement",
            "port:first",
            "port:third",
            "port:fourth"
        ]
    );
    assert_eq!(
        tcl_list::parse(result.port.get("configure.args")).unwrap(),
        ["--prefix=/opt/local", "--with-system-library"]
    );
}
const VARIANTS: &str = r#"
    PortSystem 1.0
    name app
    version 1
    depends_lib port:base
    variant tls conflicts alternative description {TLS backend} {
        depends_lib-append port:tls
    }
    variant alternative conflicts tls description {Other backend} {
        depends_lib-append port:alternative
    }
    variant compress description {Compression} { depends_lib-append port:compression }
    variant network requires tls description {Networking} { depends_lib-append port:network }
    default_variants +compress
    if {![variant_isset alternative]} { default_variants-append +tls }
"#;
#[test]
fn resolves_conditional_defaults_and_executes_only_selected_variant_bodies() {
    let result = evaluate(VARIANTS, &[]).unwrap();
    assert_eq!(result.enabled, "+compress+tls");
    assert_eq!(
        tcl_list::parse(result.port.get("depends_lib")).unwrap(),
        ["port:base", "port:tls", "port:compression"]
    );
    let metadata = tcl_list::dictionary(result.port.get("vinfo")).unwrap();
    assert_eq!(
        tcl_list::dictionary(&metadata["tls"]).unwrap()["is_default"],
        "+"
    );
    assert_eq!(result.variants["tls"].description, "TLS backend");
    let other = evaluate(VARIANTS, &["+alternative", "-compress"]).unwrap();
    assert_eq!(other.enabled, "+alternative");
    assert_eq!(other.port.get("depends_lib"), "port:base port:alternative");
}
#[test]
fn requirements_execute_before_requirer_even_when_declared_later() {
    let source = r#"name app; version 1
        variant main requires base { depends_lib-append port:main }
        variant base { depends_lib-append port:base }
    "#;
    let result = evaluate(source, &["+main"]).unwrap();
    assert_eq!(result.enabled, "+base+main");
    assert_eq!(result.port.get("depends_lib"), "port:base port:main");
    assert!(
        evaluate(source, &["+main", "-base"])
            .unwrap_err_text()
            .contains("disabled")
    );
}
trait ErrorText {
    fn unwrap_err_text(self) -> String;
}
impl ErrorText for anyhow::Result<Evaluation> {
    fn unwrap_err_text(self) -> String {
        self.err().expect("expected error").to_string()
    }
}
#[test]
fn cycles_missing_requirements_and_conflicts_fail() {
    for body in [
        "variant a requires b {}; variant b requires a {}",
        "variant a requires missing {}",
        "variant a conflicts b {}; variant b {}; default_variants +b",
    ] {
        assert!(evaluate(&format!("name app; version 1; {body}"), &["+a"]).is_err());
    }
    assert!(evaluate(VARIANTS, &["+tls", "+alternative"]).is_err());
}
#[test]
fn requested_options_override_portfile_set_append_and_replace() {
    let result = evaluate("name app; version 1; configure.args --default; configure.args-append --extra; configure.args-replace --custom --changed", &["configure.args=--custom"]).unwrap();
    assert_eq!(result.port.get("configure.args"), "--custom");
    assert!(evaluate("name app; version 1", &["unknown.option=value"]).is_err());
}
#[test]
fn retains_hooks_without_executing_their_bodies() {
    let result = evaluate(
        r#"name app; version 1
        pre-activate { exec dangerous-command }
        post-destroot { error "not metadata" }
        variant optional { error "not selected" }
    "#,
        &[],
    )
    .unwrap();
    assert_eq!(result.hooks.len(), 2);
    assert_eq!(result.hooks[0].phase, "pre-activate");
    assert!(result.hooks[0].body.contains("exec dangerous-command"));
}
#[test]
fn unsupported_commands_cannot_be_hidden_by_catch() {
    for command in [
        "source /etc/passwd",
        "socket anything 1",
        "exit",
        "load lib.dylib",
    ] {
        let source = format!("name app; version 1; catch {{{command}}}");
        let error = evaluate(&source, &[]).unwrap_err_text();
        assert!(error.contains("unsupported"), "{error}");
    }
    assert!(evaluate("name app; version 1; unknown_command", &[]).is_err());
}
#[test]
fn environment_is_explicit_and_evaluations_are_isolated() {
    let result = evaluate("name app; version 1; if {[info exists env(PATH)]} {error {inherited environment}}; options custom; custom one; set temporary yes", &[]).unwrap();
    assert_eq!(result.port.get("custom"), "one");
    assert!(evaluate("name app; version 1; set value $temporary", &[]).is_err());
}
#[test]
fn selected_subports_override_metadata_without_running_other_subports() {
    let result = evaluate(
        r#"name parent; version 1
        subport unused { error unused }
        subport app { version 2; supported_archs noarch; platforms any }
    "#,
        &[],
    )
    .unwrap();
    assert_eq!(result.port.name, "app");
    assert_eq!(result.port.get("name"), "app");
    assert_eq!(result.port.version(), "2_0");
    assert_eq!(result.port.get("supported_archs"), "noarch");
    assert!(evaluate("name parent; version 1; subport missing {}", &[]).is_err());
}
#[test]
fn evaluates_only_requested_version_and_rejects_missing_metadata() {
    assert!(evaluate("name app; version 2", &["@2_0"]).is_ok());
    assert!(evaluate("name app; version 2", &["@1"]).is_err());
    assert!(evaluate("name app", &[]).is_err());
    assert!(evaluate("PortSystem 2.0; name app; version 1", &[]).is_err());
}
#[test]
fn option_access_and_custom_options_work_inside_procedures() {
    let result = evaluate(
        r#"name app; version 1; options custom
        custom first
        proc change {} {
            if {[exists custom]} { option custom [list second third] }
        }
        change
        depends_lib [option custom]
    "#,
        &[],
    )
    .unwrap();
    assert_eq!(result.port.get("custom"), "second third");
    assert_eq!(
        tcl_list::parse(result.port.get("depends_lib")).unwrap(),
        ["second third"]
    );
}
#[test]
fn metadata_and_variants_feed_existing_archive_target_code() {
    use port_rs::{
        install::{
            Action, Item,
            target::{Architecture, Target},
        },
        source::Platform,
    };
    let evaluation = evaluate(VARIANTS, &[]).unwrap();
    let item = Item {
        port: &evaluation.port,
        variants: evaluation.enabled,
        variants_resolved: true,
        provider_fallbacks: vec![],
        requested: true,
        action: Action::Install,
    };
    let target = Target::macos(
        &Platform {
            darwin_major: 24,
            architecture: "arm",
        },
        item.port.get("platforms"),
        Architecture::Native,
    )
    .unwrap();
    assert_eq!(
        target.filename(&item).unwrap(),
        "app-1_0+compress+tls.darwin_24.arm64.tbz2"
    );
}

#[test]
fn selected_variant_requirements_are_visible_to_later_conditions() {
    let result = evaluate(
        r#"name app; version 1
        variant network requires tls {}
        variant tls {}
        if {[variant_isset tls]} { depends_lib-append port:tls }
    "#,
        &["+network"],
    )
    .unwrap();
    assert_eq!(result.enabled, "+network+tls");
    assert_eq!(result.port.get("depends_lib"), "port:tls");
}

#[test]
fn default_variant_options_and_accessors_trigger_selection() {
    let source = "name app; version 1; variant tls {}; variant other {}; default_variants +other";
    assert_eq!(
        evaluate(source, &["default_variants=+tls"])
            .unwrap()
            .enabled,
        "+tls"
    );
    let source = "name app; version 1; variant tls {}; option default_variants +tls";
    assert_eq!(evaluate(source, &[]).unwrap().enabled, "+tls");
}

#[test]
fn variant_attributes_follow_modes_and_allow_unquoted_descriptions() {
    let result = evaluate("name app; version 1; variant provides tls description Enable secure connections {}; default_variants +tls", &[]).unwrap();
    assert_eq!(result.enabled, "+tls");
    assert_eq!(
        result.variants["tls"].description,
        "Enable secure connections"
    );
    assert!(evaluate("name app; version 1; variant a b {}", &[]).is_err());
}
