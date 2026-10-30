use port_rs::{
    portfile::{self, PortGroups, Session},
    spec::PortSpec,
    tcl_list,
};
use std::collections::BTreeMap;

fn open(source: &str) -> Session {
    Session::open(
        source,
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap()
}

#[test]
fn preserves_procedures_private_variables_arrays_and_selected_variant_state() {
    let mut spec = PortSpec::named("app");
    spec.parse_modifier("+custom").unwrap();
    let mut session = Session::open(
        r#"
        name app; version 1
        set private 4
        set settings(mode) base
        proc increment {} {global private; incr private}
        variant custom {set settings(mode) selected}
        proc read_mode {} {global settings; return $settings(mode)}
    "#,
        &BTreeMap::new(),
        &spec,
        PortGroups::default(),
    )
    .unwrap();
    assert_eq!(session.invoke("increment", &[]).unwrap(), "5");
    assert_eq!(session.invoke("increment", &[]).unwrap(), "6");
    assert_eq!(session.scalar("private").unwrap(), "6");
    assert_eq!(session.invoke("read_mode", &[]).unwrap(), "selected");
    assert_eq!(session.metadata().enabled, "+custom");
    assert!(!session.metadata().port.fields.contains_key("private"));
}

#[test]
fn deferred_target_wrappers_share_live_context_and_preserve_declaration_order() {
    let mut session = open(
        r#"
        name app; version 1
        set order {}
        set counter 0
        proc record {label} {global order counter; incr counter; lappend order $label}
        pre-build {record before1}
        pre-build {record before2}
        build {record body; version 2}
        post-build {record after}
    "#,
    );
    assert_eq!(session.scalar("counter").unwrap(), "0");
    let item = session
        .metadata()
        .dependency_items
        .values()
        .find(|item| item.get("name").map(String::as_str) == Some("org.macports.build"))
        .unwrap()
        .clone();
    // Explicitly exercise only the user wrappers; native initialization and
    // prerun/postrun callbacks are the responsibility of the future scheduler.
    let mut callbacks = tcl_list::parse(&item["pre"]).unwrap();
    callbacks.push(item["procedure"].clone());
    callbacks.extend(tcl_list::parse(&item["post"]).unwrap());
    for callback in callbacks {
        assert_eq!(
            session.invoke(&callback, &["org.macports.build"]).unwrap(),
            "0"
        );
    }
    assert_eq!(
        session.scalar("order").unwrap(),
        "before1 before2 body after"
    );
    assert_eq!(session.scalar("counter").unwrap(), "4");
    assert_eq!(session.scalar("version").unwrap(), "2");
    assert_eq!(session.metadata().port.get("version"), "1");
}

#[test]
fn invocation_arguments_remain_data_and_errors_do_not_leak_local_scope() {
    let mut session = open(
        r#"
        name app; version 1
        set touched no
        proc echo {value} {return $value}
        proc broken {} {set local secret; error broken}
    "#,
    );
    let literal = "[set touched yes]; $version\n{unclosed";
    assert_eq!(session.invoke("echo", &[literal]).unwrap(), literal);
    assert_eq!(session.scalar("touched").unwrap(), "no");
    assert!(
        session
            .invoke("broken", &[])
            .unwrap_err()
            .to_string()
            .contains("broken")
    );
    assert!(session.scalar("local").is_err());
    assert_eq!(
        session.invoke("echo", &["still usable"]).unwrap(),
        "still usable"
    );
}

#[test]
fn caught_unsupported_operations_and_failed_group_imports_remain_fatal() {
    for body in ["catch {exec must-not-run}", "catch {PortGroup missing 1.0}"] {
        let mut session = open(&format!("name app; version 1; proc bad {{}} {{{body}}}"));
        assert!(session.invoke("bad", &[]).is_err());
        assert!(session.invoke("set", &["version", "2"]).is_err());
    }
}

#[test]
fn retained_portgroups_and_lazy_defaults_survive_metadata_snapshot() {
    let mut groups = PortGroups::default();
    groups
        .insert("helper", "1.0", "proc suffix {} {return -helper}".into())
        .unwrap();
    let mut session = Session::open(
        r#"
        name app; version 1; PortGroup helper 1.0
        default custom {[suffix]-${version}}
        proc update {} {global custom; version 2; return $custom}
    "#,
        &BTreeMap::new(),
        &PortSpec::named("app"),
        groups,
    )
    .unwrap();
    assert_eq!(session.invoke("update", &[]).unwrap(), "-helper-2");
    assert_eq!(session.metadata().groups.len(), 1);
    let plain = portfile::evaluate(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
    )
    .unwrap();
    assert_eq!(plain.port.get("version"), "1");
}

#[test]
fn local_tree_sessions_keep_imported_procedures_and_remain_isolated() {
    let tree = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tree.path().join("net/app")).unwrap();
    std::fs::create_dir_all(tree.path().join("_resources/port1.0/group")).unwrap();
    std::fs::write(
        tree.path().join("net/app/Portfile"),
        "name app; version 1; PortGroup helper 1.0; set private 0",
    )
    .unwrap();
    std::fs::write(
        tree.path().join("_resources/port1.0/group/helper-1.0.tcl"),
        "proc increment {} {global private; incr private}",
    )
    .unwrap();
    let port = port_rs::index::Port {
        name: "app".into(),
        fields: BTreeMap::from([("portdir".into(), "net/app".into())]),
    };
    let mut sessions = (0..2)
        .map(|_| {
            portfile::open_from_tree(
                tree.path(),
                &port,
                &BTreeMap::new(),
                &PortSpec::named("app"),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(sessions[0].invoke("increment", &[]).unwrap(), "1");
    assert_eq!(sessions[1].scalar("private").unwrap(), "0");
    assert_eq!(sessions[1].invoke("increment", &[]).unwrap(), "1");
    assert_eq!(sessions[0].invoke("increment", &[]).unwrap(), "2");
    assert_eq!(sessions[0].metadata().groups.len(), 1);
}

#[test]
fn renamed_global_aliases_preserve_lazy_default_reads_writes_and_unsets() {
    let mut session = open(
        r#"
        name app; version 1
        proc read_alias {} {upvar #0 distname local; return $local}
        proc write_alias {} {upvar #0 distname local; set local explicit}
        proc unset_alias {} {upvar #0 distname local; unset local; set local recreated}
    "#,
    );
    assert_eq!(session.invoke("read_alias", &[]).unwrap(), "app-1");
    session.invoke("version", &["2"]).unwrap();
    assert_eq!(session.invoke("read_alias", &[]).unwrap(), "app-2");
    session.invoke("write_alias", &[]).unwrap();
    assert_eq!(session.scalar("distname").unwrap(), "explicit");
    session.invoke("unset_alias", &[]).unwrap();
    assert_eq!(session.scalar("distname").unwrap(), "recreated");
}

#[test]
fn qualified_global_build_options_resolve_defaults_and_keep_local_shadows() {
    let mut session = open(
        r#"
        name app; version 1
        proc read_build {} {
            set distname local
            return [list $distname $::distname ${::build.jobs}]
        }
        proc change_build {} {set ::version 2; set ::build.jobs 7}
    "#,
    );
    assert_eq!(session.invoke("read_build", &[]).unwrap(), "local app-1 2");
    session.invoke("change_build", &[]).unwrap();
    assert_eq!(session.invoke("read_build", &[]).unwrap(), "local app-2 7");
    session
        .invoke(
            "proc",
            &["clear", "", "unset ::distname; set ::distname explicit"],
        )
        .unwrap();
    session.invoke("clear", &[]).unwrap();
    assert_eq!(session.scalar("distname").unwrap(), "explicit");
}
