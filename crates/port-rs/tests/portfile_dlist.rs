use port_rs::{portfile, spec::PortSpec};
use std::collections::BTreeMap;
fn run(body: &str) -> portfile::Evaluation {
    portfile::evaluate(
        &format!(
            r#"
        name app; version 1
        set events {{}}
        proc item {{name provides requires {{uses {{}}}}}} {{
            set item [ditem_create]
            ditem_key $item name $name
            ditem_key $item provides $provides
            ditem_key $item requires $requires
            ditem_key $item uses $uses
            return $item
        }}
        proc execute {{item}} {{global events; lappend events [ditem_key $item name]; return 0}}
        {body}
    "#
        ),
        &BTreeMap::new(),
        &PortSpec::named("app"),
    )
    .unwrap()
}
#[test]
fn required_closure_and_execution_order_are_distinct_and_deduplicate_shared_items() {
    let result = run(r#"
        set base [item base base {}]
        set left [item left left base]
        set right [item right right base]
        set root [item root root {left right}]
        set unused [item unused unused {}]
        set all [list $base $left $right $root $unused]
        set selected [dlist_append_dependents $all $root {}]
        set names {}
        foreach entry $selected {lappend names [ditem_key $entry name]}
        description {*}$names
        set remaining [dlist_eval $selected {} execute]
        if {$remaining ne {}} {error remaining}
        long_description {*}$events
    "#);
    assert_eq!(result.port.get("description"), "root left base right");
    assert_eq!(result.port.get("long_description"), "base left right root");
}
#[test]
fn optional_dependencies_delay_pending_providers_and_prefer_fewer_unmet_tokens() {
    let result = run(r#"
        set consumer [item consumer consumer {} provider]
        set absent [item absent absent {} unavailable]
        set provider [item provider provider {}]
        set ready [item ready ready {}]
        set remaining [dlist_eval [list $consumer $absent $provider $ready] {} execute]
        description {*}$events
        long_description {*}$dlist_eval_reason
    "#);
    assert_eq!(
        result.port.get("description"),
        "provider consumer ready absent"
    );
    assert_eq!(result.port.get("long_description"), "");
}
#[test]
fn cycles_and_missing_requirements_remain_pending_without_running_handlers() {
    let result = run(r#"
        set a [item a a b]
        set b [item b b a]
        set missing [item missing missing unavailable]
        set closure [dlist_append_dependents [list $a $b] $a {}]
        if {[llength $closure] != 2} {error cycle}
        set pending [dlist_eval [list $a $b $missing] {} execute]
        description {*}[list [llength $pending] $dlist_eval_reason $events]
    "#);
    assert_eq!(result.port.get("description"), "3 unmet_deps {}");
}
#[test]
fn handler_prefixes_failure_statuses_and_can_fail_follow_native_rules() {
    let result = run(r#"
        set a [item a a {}]
        set b [item b b a]
        set c [item c c {}]
        proc fail {prefix item} {global events; lappend events "$prefix:[ditem_key $item name]"; return 4}
        set pending [dlist_eval [list $a $b $c] {} {fail failed}]
        description {*}[list [llength $pending] $dlist_eval_reason $events]
        set events {}
        set pending [dlist_eval [list $a $b $c] {} {fail failed} 1]
        long_description {*}[list [llength $pending] $dlist_eval_reason $events]
    "#);
    assert_eq!(result.port.get("description"), "3 handler failed:a");
    assert_eq!(
        result.port.get("long_description"),
        "1 unmet_deps {failed:a failed:c}"
    );
}
#[test]
fn handler_exceptions_stop_even_when_can_fail_and_nonnumeric_status_is_failure() {
    let result = run(r#"
        set a [item a a {}]
        proc broken {item} {error failed}
        proc invalid {item} {return invalid}
        set result [dlist_eval [list $a] {} broken 1]
        description {*}[list [llength $result] $dlist_eval_reason]
        set result [dlist_eval [list $a] {} invalid]
        long_description {*}[list [llength $result] $dlist_eval_reason]
    "#);
    assert_eq!(result.port.get("description"), "1 handler");
    assert_eq!(result.port.get("long_description"), "1 handler");
}
#[test]
fn test_condition_marks_tokens_and_custom_selector_can_upvar_local_status() {
    let result = run(r#"
        set a [item a a {}]
        set b [item b b a]
        proc done {item} {return [expr {[ditem_key $item name] eq "a"}]}
        proc selector {items status} {
            upvar $status local
            if {$local(a) != 1} {error status}
            return [dlist_get_next $items local]
        }
        proc schedule {items} {
            set reason untouched
            set pending [dlist_eval $items done execute 0 selector reason]
            return [list $pending $reason]
        }
        description {*}[schedule [list $b $a]]
        long_description {*}$events
        if {[info exists statusdict] || [info exists reason]} {error leaked}
    "#);
    assert_eq!(result.port.get("description"), "{} {}");
    assert_eq!(result.port.get("long_description"), "b");
}
#[test]
fn list_helpers_preserve_literal_tokens_and_modify_callers_list() {
    let result = run(r#"
        set token {a; [error injected]}
        set a [item a [list $token] {}]
        set items [list $a $a]
        if {[llength [dlist_search $items provides $token]] != 2} {error search}
        if {![dlist_has_pending $items [list $token]]} {error pending}
        set status($token) 1.0
        if {[dlist_count_unmet $items status [list $token missing]] != 1} {error count}
        proc remove {value} {set items [list $value $value]; dlist_delete items $value; return $items}
        description {*}[llength [remove $a]]
        long_description {*}[llength $items]
    "#);
    assert_eq!(result.port.get("description"), "1");
    assert_eq!(result.port.get("long_description"), "2");
}

#[test]
fn deep_required_closures_are_iterative_and_keep_all_providers() {
    let result = run(r#"
        set items {}
        set previous {}
        for {set i 0} {$i < 5000} {incr i} {
            set current [item n$i t$i $previous]
            lappend items $current
            set previous t$i
        }
        set closure [dlist_append_dependents $items $current {}]
        description {*}[llength $closure]
        set first [item first shared {}]
        set second [item second shared {}]
        set root [item root root shared]
        long_description {*}[llength [dlist_append_dependents [list $first $second $root] $root {}]]
    "#);
    assert_eq!(result.port.get("description"), "5000");
    assert_eq!(result.port.get("long_description"), "3");
}
