use port_rs::{
    portfile::{self, Evaluation, PortGroups},
    spec::PortSpec,
};
use std::collections::BTreeMap;

fn evaluate(source: &str, modifiers: &[&str]) -> anyhow::Result<Evaluation> {
    let mut spec = PortSpec::named("app");
    for modifier in modifiers {
        spec.parse_modifier(modifier)?;
    }
    portfile::evaluate(source, &BTreeMap::new(), &spec)
}

#[test]
fn custom_targets_are_deferred_without_substituting_or_executing_bodies() {
    let result = evaluate(
        r#"
        name app
        version 1
        configure {}
        build {system "make ${build.target}"; exec must-not-run}
        destroot {error "must not run during metadata evaluation"}
    "#,
        &[],
    )
    .unwrap();
    assert_eq!(result.targets["configure"], "");
    assert_eq!(
        result.targets["build"],
        "system \"make ${build.target}\"; exec must-not-run"
    );
    assert_eq!(result.targets.len(), 3);
    assert!(!result.targets.contains_key("fetch"));
}

#[test]
fn last_override_wins_while_hooks_accumulate_in_declaration_order() {
    let mut groups = PortGroups::default();
    groups
        .insert(
            "helper",
            "1.0",
            "pre-build {group-before}; build {group-build}; post-build {group-after}".into(),
        )
        .unwrap();
    let result = portfile::evaluate_with_groups(
        r#"
        name app
        version 1
        PortGroup helper 1.0
        pre-build {port-before}
        build {port-build}
        post-build {port-after}
        build {}
    "#,
        &BTreeMap::new(),
        &PortSpec::named("app"),
        groups,
    )
    .unwrap();
    assert_eq!(result.targets["build"], "");
    assert_eq!(
        result
            .hooks
            .iter()
            .map(|hook| (hook.phase.as_str(), hook.body.as_str()))
            .collect::<Vec<_>>(),
        [
            ("pre-build", "group-before"),
            ("post-build", "group-after"),
            ("pre-build", "port-before"),
            ("post-build", "port-after"),
        ]
    );
}

#[test]
fn only_matching_subports_platforms_and_selected_variants_override_targets() {
    let script = r#"
        name app
        version 1
        build {base}
        variant custom { build {custom}; post-build {custom-after} }
        subport unrelated { build {wrong-subport} }
        platform linux { build {wrong-platform} }
    "#;
    let env = BTreeMap::from([
        ("os.platform".into(), "darwin".into()),
        ("os.subplatform".into(), "macosx".into()),
        ("os.arch".into(), "arm".into()),
        ("os.major".into(), "24".into()),
    ]);
    let result = portfile::evaluate(script, &env, &PortSpec::named("app")).unwrap();
    assert_eq!(result.targets["build"], "base");
    assert!(result.hooks.is_empty());
    let mut spec = PortSpec::named("app");
    spec.parse_modifier("+custom").unwrap();
    let result = portfile::evaluate(script, &env, &spec).unwrap();
    assert_eq!(result.targets["build"], "custom");
    assert_eq!(result.hooks[0].body, "custom-after");
}

#[test]
fn declarations_inside_procedures_preserve_body_and_normal_argument_substitution() {
    let result = evaluate(
        r#"
        name app
        version 1
        proc setup {} {
            set command {system "make all"}
            build $command
            destroot {system "make DESTDIR=${destroot} install"}
        }
        setup
    "#,
        &[],
    )
    .unwrap();
    assert_eq!(result.targets["build"], "system \"make all\"");
    assert_eq!(
        result.targets["destroot"],
        "system \"make DESTDIR=${destroot} install\""
    );
}

#[test]
fn target_and_hook_arity_errors_do_not_replace_valid_declarations() {
    let result = evaluate(
        r#"
        name app
        version 1
        build {original}
        pre-build {before}
        foreach script {{build} {build first second} {pre-build} {post-build first second}} {
            if {![catch $script]} {error "invalid declaration accepted"}
        }
    "#,
        &[],
    )
    .unwrap();
    assert_eq!(result.targets["build"], "original");
    assert_eq!(result.hooks.len(), 1);
}

#[test]
fn checksum_and_archive_targets_preserve_their_hooks() {
    let result = evaluate(r#"
        name app
        version 1
        foreach phase {fetch checksum extract patch configure build destroot install activate deactivate uninstall test archive unarchive clean} {
            $phase {custom}
            pre-$phase {before}
            post-$phase {after}
        }
    "#, &[]).unwrap();
    assert_eq!(result.targets.len(), 15);
    assert_eq!(result.hooks.len(), 30);
    for phase in result.targets.keys() {
        assert!(
            result
                .hooks
                .iter()
                .any(|hook| hook.phase == format!("pre-{phase}"))
        );
        assert!(
            result
                .hooks
                .iter()
                .any(|hook| hook.phase == format!("post-{phase}"))
        );
    }
}
