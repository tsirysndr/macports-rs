use port_rs::{
    portfile::{self, Evaluation},
    spec::PortSpec,
    tcl_list,
};
use std::collections::BTreeMap;

fn run(script: &str) -> anyhow::Result<Evaluation> {
    portfile::evaluate(
        &format!("name app; version 1; {script}"),
        &BTreeMap::new(),
        &PortSpec::named("app"),
    )
}

#[test]
fn items_preserve_fields_lists_and_membership() {
    let result = run(r#"
        set item [ditem_create]
        ditem_key $item name custom
        if {[ditem_key $item missing] ne ""} {error missing}
        ditem_append $item requires {two words} first first
        ditem_append_unique $item requires first second
        if {![ditem_contains $item requires {two words}]} {error membership}
        if {![ditem_contains $item requires]} {error existence}
        if {[ditem_contains $item absent]} {error absence}
        if {[ditem_as_string $item] ne [ditem_key $item]} {error snapshot}
        description [ditem_key $item requires]
    "#)
    .unwrap();
    assert_eq!(
        tcl_list::parse(result.port.get("description")).unwrap(),
        ["two words", "first", "second"]
    );
    assert!(
        result
            .dependency_items
            .values()
            .any(|fields| fields.get("name").is_some_and(|name| name == "custom"))
    );
}

#[test]
fn unique_append_keeps_its_native_cache_and_delete_removes_it() {
    let result = run(r#"
        set item [ditem_create]
        ditem_append_unique $item values a b a
        ditem_key $item values replaced
        description [ditem_append_unique $item values b c]
        ditem_delete $item
        if {[ditem_key $item] ne ""} {error deleted}
        if {![catch {ditem_key $item name}]} {error missing}
        ditem_key $item values fresh
        notes [ditem_append_unique $item values final]
    "#)
    .unwrap();
    assert_eq!(result.port.get("description"), "a b c");
    assert_eq!(result.port.get("notes"), "fresh final");
}

#[test]
fn custom_targets_record_properties_without_invoking_procedures() {
    let result = run(r#"
        proc must_not_run {} {error executed}
        set item [target_new org.custom.stage must_not_run]
        target_requires $item fetch checksum
        target_requires $item extract
        target_uses $item compiler
        target_prerun $item before
        target_postrun $item after
        target_init $item setup
        target_deplist $item needs
        target_runtype $item always
        target_state $item no
        target_runpkg $item custom_runtime
        description [lindex ${portutil::targets} [expr {[llength ${portutil::targets}] - 1}]]
    "#)
    .unwrap();
    let fields = &result.dependency_items[result.port.get("description")];
    assert_eq!(fields["procedure"], "must_not_run");
    assert_eq!(fields["requires"], "fetch checksum extract");
    assert_eq!(fields["runpkg"], "custom_runtime");
    assert_eq!(fields["prerun"], "before");
    assert_eq!(fields["postrun"], "after");
    assert_eq!(fields["init"], "setup");
    assert_eq!(fields["state"], "no");
}

#[test]
fn builtin_target_handles_expose_metadata_and_track_registered_hooks() {
    let result = run(r#"
        set item ${org.macports.destroot}
        if {[ditem_key $item procedure] ne "portdestroot::destroot_main"} {error procedure}
        if {![ditem_contains $item requires build]} {error requires}
        pre-destroot {revision 2}
        pre-destroot {revision 3}
        post-destroot {revision 4}
        destroot {revision 5}
        if {[llength [ditem_key $item pre]] != 2} {error pre}
        if {[llength [ditem_key $item post]] != 1} {error post}
        if {${revision} != 0} {error executed}
        set handler [ditem_key $item procedure]
        if {[$handler ignored] != 0} {error status}
        description [ditem_key $item name]
    "#)
    .unwrap();
    assert_eq!(result.port.get("revision"), "5");
    assert_eq!(result.port.get("description"), "org.macports.destroot");
    assert_eq!(result.hooks.len(), 3);
    assert_eq!(result.targets["destroot"], "revision 5");
}

#[test]
fn user_procedures_import_globals_at_invocation_and_preserve_errors() {
    let result = run(r#"
        makeuserproc deferred {description $later}
        set later {available later}
        deferred
        build {error expected-failure}
        set handler [ditem_key ${org.macports.build} procedure]
        if {![catch {$handler ignored} message]} {error missing-error}
        notes $message
    "#)
    .unwrap();
    assert_eq!(result.port.get("description"), "available later");
    assert_eq!(result.port.get("notes"), "expected-failure");
}

#[test]
fn malformed_lists_and_arity_errors_leave_item_values_intact() {
    let result = run(r#"
        set item [ditem_create]
        ditem_key $item broken \{
        if {![catch {ditem_append $item broken next}]} {error malformed}
        if {[ditem_key $item broken] ne "\{"} {error changed}
        foreach command {{ditem_create extra} {ditem_delete} {ditem_contains} {ditem_append} {target_new} {target_runpkg item} {makeuserproc}} {
            if {![catch $command]} {error arity}
        }
        description ok
    "#).unwrap();
    assert_eq!(result.port.get("description"), "ok");
}

#[test]
fn target_names_are_data_when_generating_wrapper_procedures() {
    let result = run(r#"
        ditem_key ${org.macports.build} name {odd; [error injected] name}
        build {revision 9}
        set handler [ditem_key ${org.macports.build} procedure]
        $handler ignored
    "#)
    .unwrap();
    assert_eq!(result.port.get("revision"), "9");
}
