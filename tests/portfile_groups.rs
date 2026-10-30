use port_rs::{
    portfile::{self, Evaluation, PortGroups},
    spec::PortSpec,
    tcl_list,
};
use std::collections::BTreeMap;
fn run(source: &str, groups: PortGroups) -> anyhow::Result<Evaluation> {
    portfile::evaluate_with_groups(source, &BTreeMap::new(), &PortSpec::named("app"), groups)
}
fn repository(entries: &[(&str, &str, &str)]) -> PortGroups {
    let mut groups = PortGroups::default();
    for (name, version, source) in entries {
        groups.insert(name, version, (*source).into()).unwrap();
    }
    groups
}
#[test]
fn nested_and_repeated_groups_execute_once_in_the_same_interpreter() {
    let groups = repository(&[
        (
            "outer",
            "1.0",
            "PortGroup shared 1.0; depends_lib-append port:outer; options outer.flag; outer.flag enabled",
        ),
        (
            "shared",
            "1.0",
            "depends_lib-append port:shared; variant tls { depends_lib-append port:tls }; default_variants +tls",
        ),
    ]);
    let result = run(
        "name app; version 1; PortGroup outer 1.0; PortGroup shared 1.0; PortGroup outer 1.0",
        groups,
    )
    .unwrap();
    assert_eq!(
        result.port.get("depends_lib"),
        "port:shared port:outer port:tls"
    );
    assert_eq!(result.port.get("outer.flag"), "enabled");
    assert_eq!(result.enabled, "+tls");
    assert_eq!(
        result
            .groups
            .iter()
            .map(|group| group.name.as_str())
            .collect::<Vec<_>>(),
        ["outer", "shared"]
    );
    for group in &result.groups {
        assert!(group.path.is_none());
        assert!(group.size > 0);
        assert_eq!(group.sha256.len(), 64);
    }
}
#[test]
fn recursive_imports_terminate_like_upstream_pre_registered_includes() {
    let groups = repository(&[
        ("a", "1.0", "PortGroup b 1.0; depends_lib-append port:a"),
        ("b", "1.0", "PortGroup a 1.0; depends_lib-append port:b"),
    ]);
    let result = run("name app; version 1; PortGroup a 1.0", groups).unwrap();
    assert_eq!(result.port.get("depends_lib"), "port:b port:a");
    assert_eq!(result.groups.len(), 2);
}
#[test]
fn different_versions_are_distinct_imports() {
    let groups = repository(&[
        ("helper", "1.0", "depends_lib-append port:one"),
        ("helper", "2.0", "depends_lib-append port:two"),
    ]);
    let result = run(
        "name app; version 1; PortGroup helper 1.0; PortGroup helper 2.0; PortGroup helper 1.0",
        groups,
    )
    .unwrap();
    assert_eq!(result.port.get("depends_lib"), "port:one port:two");
    assert_eq!(result.groups.len(), 2);
    assert_ne!(result.groups[0].sha256, result.groups[1].sha256);
}
#[test]
fn declared_group_options_accept_user_overrides_before_group_loading() {
    let groups = repository(&[(
        "helper",
        "1.0",
        "options helper.flag; helper.flag default; helper.flag-append extra",
    )]);
    let mut spec = PortSpec::named("app");
    spec.parse_modifier("helper.flag=custom").unwrap();
    let result = portfile::evaluate_with_groups(
        "name app; version 1; PortGroup helper 1.0",
        &BTreeMap::new(),
        &spec,
        groups,
    )
    .unwrap();
    assert_eq!(result.port.get("helper.flag"), "custom");
    spec.parse_modifier("misspelled.option=value").unwrap();
    assert!(
        portfile::evaluate_with_groups(
            "name app; version 1",
            &BTreeMap::new(),
            &spec,
            PortGroups::default()
        )
        .is_err()
    );
}
#[test]
fn imported_hooks_remain_deferred() {
    let result = run(
        "name app; version 1; PortGroup hook 1.0",
        repository(&[(
            "hook",
            "1.0",
            "pre-activate { exec no-external-commands }; post-destroot { error not-now }",
        )]),
    )
    .unwrap();
    assert_eq!(result.hooks.len(), 2);
    assert_eq!(result.hooks[0].phase, "pre-activate");
}
#[test]
fn caught_missing_or_failed_imports_cannot_return_partial_metadata() {
    for group in [
        PortGroups::default(),
        repository(&[(
            "broken",
            "1.0",
            "depends_lib-append port:partial; error broken",
        )]),
        repository(&[("broken", "1.0", "PortGroup missing 1.0")]),
    ] {
        let error = run(
            "name app; version 1; catch {PortGroup broken 1.0}; catch {PortGroup broken 1.0}",
            group,
        )
        .err()
        .unwrap()
        .to_string();
        assert!(error.contains("imports did not complete"), "{error}");
    }
    assert!(
        run(
            "name app; version 1; PortGroup broken 1.0",
            repository(&[("broken", "1.0", "exec forbidden")])
        )
        .is_err()
    );
}
#[test]
fn skipped_conditionals_and_unselected_subports_do_not_load_groups() {
    let result = run("name app; version 1; if {0} { PortGroup missing 1.0 }; subport unused { PortGroup absent 1.0 }", PortGroups::default()).unwrap();
    assert!(result.groups.is_empty());
}
#[test]
fn filesystem_search_precedence_and_embedded_overrides_are_deterministic() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    std::fs::write(first.join("helper-1.0.tcl"), "revision 1").unwrap();
    std::fs::write(second.join("helper-1.0.tcl"), "revision 2").unwrap();
    std::fs::write(second.join("only-1.0.tcl"), "depends_lib port:only").unwrap();
    let groups = PortGroups::from_directories(vec![first.clone(), second.clone()]);
    let result = run(
        "name app; version 1; PortGroup helper 1.0; PortGroup only 1.0",
        groups.clone(),
    )
    .unwrap();
    assert_eq!(result.port.version(), "1_1");
    assert_eq!(
        result.groups[0].path.as_ref(),
        Some(&first.join("helper-1.0.tcl"))
    );
    assert_eq!(result.groups[0].size, 10);
    assert_eq!(
        result.groups[1].path.as_ref(),
        Some(&second.join("only-1.0.tcl"))
    );
    let mut groups = groups;
    groups.insert("helper", "1.0", "revision 3".into()).unwrap();
    let result = run("name app; version 1; PortGroup helper 1.0", groups).unwrap();
    assert_eq!(result.port.version(), "1_3");
    assert!(result.groups[0].path.is_none());
}
#[test]
fn groups_export_native_portinfo_records_to_imported_code() {
    let result = run(
        r#"name app; version 1; PortGroup helper 1.0
        options captured
        captured {*}$PortInfo(portgroups)
    "#,
        repository(&[("helper", "1.0", "revision 2")]),
    )
    .unwrap();
    let records = tcl_list::parse(result.port.get("captured")).unwrap();
    assert_eq!(
        tcl_list::parse(&records[0]).unwrap(),
        ["helper", "1.0", "<embedded>"]
    );
}
#[test]
fn invalid_group_components_and_oversized_or_non_utf8_sources_fail() {
    let mut groups = PortGroups::default();
    for name in ["../other", "/tmp/other", "..", ".", "a\\b", "a\n"] {
        assert!(groups.insert(name, "1.0", String::new()).is_err());
    }
    assert!(
        groups
            .insert("helper", "../../other", String::new())
            .is_err()
    );
    assert!(
        groups
            .insert("huge", "1.0", "x".repeat(4 * 1024 * 1024 + 1))
            .is_err()
    );
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("invalid-1.0.tcl"), [255, 255]).unwrap();
    let file = std::fs::File::create(temp.path().join("huge-1.0.tcl")).unwrap();
    file.set_len(4 * 1024 * 1024 + 1).unwrap();
    std::fs::create_dir(temp.path().join("directory-1.0.tcl")).unwrap();
    for name in ["invalid", "huge", "directory", "../escape"] {
        assert!(
            run(
                &format!("name app; version 1; PortGroup {name} 1.0"),
                PortGroups::from_directories(vec![temp.path().into()])
            )
            .is_err()
        );
    }
}
