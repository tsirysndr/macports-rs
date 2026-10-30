use port_rs::{
    portfile::{self, Evaluation},
    spec::PortSpec,
    tcl_list,
};
use std::collections::BTreeMap;
fn run(script: &str) -> anyhow::Result<Evaluation> {
    portfile::evaluate(
        &format!("name app; version 1\n{script}"),
        &BTreeMap::new(),
        &PortSpec::named("app"),
    )
}
#[test]
fn distfiles_follow_name_version_and_extraction_format_when_read() {
    let result = run(r#"
        options first second third
        first $distfiles
        version 2
        use_xz yes
        second [set distfiles]
        name renamed
        third $distfiles
        name app
    "#)
    .unwrap();
    assert_eq!(result.port.get("first"), "app-1.tar.gz");
    assert_eq!(result.port.get("second"), "app-2.tar.xz");
    assert_eq!(result.port.get("third"), "renamed-2.tar.xz");
    assert_eq!(result.port.get("distfiles"), "app-2.tar.xz");
}
#[test]
fn explicit_values_override_defaults_and_later_default_declarations() {
    let result = run(r#"
        distfiles fixed.tar
        default distfiles {$name-wrong.tar}
        version 2
        options custom
        custom explicit
        default custom {wrong}
    "#)
    .unwrap();
    assert_eq!(result.port.get("distfiles"), "fixed.tar");
    assert_eq!(result.port.get("custom"), "explicit");
}
#[test]
fn direct_set_append_and_unset_remove_lazy_defaults() {
    let result = run(r#"
        options custom other
        default custom {$name-initial}
        append custom -tail
        name changed
        other $custom
        name app
        unset custom
        default custom {$version-restored}
        set custom fixed
        version 2
    "#)
    .unwrap();
    assert_eq!(result.port.get("other"), "app-initial-tail");
    assert_eq!(result.port.get("custom"), "fixed");
}
#[test]
fn defaults_can_be_reregistered_until_explicitly_assigned() {
    let result =
        run("options custom; default custom {$name-first}; default custom {$name-second}").unwrap();
    assert_eq!(result.port.get("custom"), "app-second");
}
#[test]
fn default_substitution_uses_global_scope_without_replacing_caller_locals() {
    let result = run(r#"
        options custom observed
        default custom {$name-${version}}
        proc inspect_default {} {
            set name local
            set version local-version
            global custom
            set value $custom
            if {$name ne "local" || $version ne "local-version"} { error {local scope changed} }
            return $value
        }
        observed [inspect_default]
    "#)
    .unwrap();
    assert_eq!(result.port.get("observed"), "app-1");
}
#[test]
fn local_variables_do_not_trigger_global_default_hooks() {
    let result = run(r#"
        options custom observed
        default custom {$name-default}
        proc local {} { set custom own; append custom -value; unset custom; return ok }
        observed [local]
    "#)
    .unwrap();
    assert_eq!(result.port.get("custom"), "app-default");
    assert_eq!(result.port.get("observed"), "ok");
}
#[test]
fn defaults_resolve_in_expressions_and_global_alias_writes_cancel_them() {
    let result = run(r#"
        options count observed
        default count {[expr {$version + 2}]}
        if {$count != 3} { error wrong-value }
        proc replace {} { global count; set count 10 }
        replace
        version 2
        observed $count
    "#)
    .unwrap();
    assert_eq!(result.port.get("observed"), "10");
}
#[test]
fn templates_are_data_outside_explicit_substitutions() {
    let result = run(r#"options custom; default custom {"$name"; {[list $version]}}"#).unwrap();
    assert_eq!(result.port.get("custom"), "\"app\"; {1}");
    let result = run(r#"options custom; default custom {literal\ntext}"#).unwrap();
    assert_eq!(result.port.get("custom"), r"literal\ntext");
}
#[test]
fn circular_defaults_fail_without_poisoning_subsequent_reads() {
    assert!(run("options a b; default a {$b}; default b {$a}; set value $a").is_err());
    let result =
        run("options a b; default a {$b}; default b {$a}; catch {set value $a}; set b repaired")
            .unwrap();
    assert_eq!(result.port.get("a"), "repaired");
}
#[test]
fn errors_during_final_default_resolution_are_not_silently_dropped() {
    assert!(run("options custom; default custom {$missing_variable}").is_err());
    assert!(run("options custom; default custom {[error broken]}").is_err());
    assert!(run("options custom; default custom {[catch {exec forbidden}]}").is_err());
}
#[test]
fn extraction_formats_behave_like_radio_buttons_and_honor_explicit_suffix() {
    let result = run(r#"
        options first second third
        use_bzip2 yes
        first $distfiles
        set use_zip yes
        second $distfiles
        if {[info exists use_bzip2]} { error {old format is still selected} }
        unset use_zip
        third $distfiles
        extract.suffix .custom
        use_xz yes
    "#)
    .unwrap();
    assert_eq!(result.port.get("first"), "app-1.tar.bz2");
    assert_eq!(result.port.get("second"), "app-1.zip");
    assert_eq!(result.port.get("third"), "app-1.tar.gz");
    assert_eq!(result.port.get("distfiles"), "app-1.custom");
}
#[test]
fn distfiles_are_proper_lists_even_when_distname_contains_whitespace() {
    let result = run("distname {source name}; use_xz yes").unwrap();
    assert_eq!(
        tcl_list::parse(result.port.get("distfiles")).unwrap(),
        ["source name.tar.xz"]
    );
}
#[test]
fn defaults_preserve_user_options_before_evaluation() {
    let mut spec = PortSpec::named("app");
    spec.parse_modifier("distfiles=custom.tar").unwrap();
    let result =
        portfile::evaluate("name app; version 1; use_xz yes", &BTreeMap::new(), &spec).unwrap();
    assert_eq!(result.port.get("distfiles"), "custom.tar");
}

#[test]
fn appending_to_a_broken_default_reports_the_read_error() {
    assert!(run("options custom; default custom {$missing}; custom-append tail").is_err());
    assert_eq!(
        run("options custom; default custom {$missing}; custom replacement")
            .unwrap()
            .port
            .get("custom"),
        "replacement"
    );
}

#[test]
fn deep_acyclic_default_chains_hit_the_interpreter_limit() {
    let mut script = String::new();
    for index in 0..500 {
        script.push_str(&format!("default value{index} {{$value{}}}\n", index + 1));
    }
    script.push_str("set value500 done\nset result $value0\n");
    let error = run(&script).err().unwrap();
    assert!(error.to_string().contains("too many nested"), "{error:#}");
}

#[test]
fn interpreter_mutators_preserve_default_read_failures() {
    for command in [
        "append custom suffix",
        "lappend custom suffix",
        "dict set custom key value",
        "dict unset custom key",
        "incr custom",
    ] {
        let source = format!("options custom; default custom {{$missing}}; {command}");
        assert!(run(&source).is_err(), "{command}");
    }
}
