use port_rs::{
    dependencies::provider::{Resolver, Search},
    registry::{
        Registry,
        write::{ActivationFile, ImageFile, NewImage, Writer},
    },
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};
fn search(root: &Path) -> Search {
    Search {
        prefix: root.into(),
        frameworks_dir: root.join("Frameworks"),
        platform: "darwin".into(),
        environment: BTreeMap::from([(
            "PATH".into(),
            format!(
                "{}:{}",
                root.join("first").display(),
                root.join("second").display()
            ),
        )]),
    }
}
#[test]
fn probes_literal_paths_executables_and_versioned_libraries() {
    let root = tempfile::tempdir().unwrap();
    for dir in ["first", "second", "lib", "Frameworks"] {
        fs::create_dir(root.path().join(dir)).unwrap();
    }
    let resolver = Resolver::new(None, search(root.path())).unwrap();
    assert_eq!(
        resolver.resolve("bin:tool:fallback").unwrap(),
        Some("fallback".into())
    );
    fs::write(root.path().join("first/tool"), b"not executable").unwrap();
    fs::set_permissions(
        root.path().join("first/tool"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert_eq!(
        resolver.resolve("bin:tool:fallback").unwrap(),
        Some("fallback".into())
    );
    fs::write(root.path().join("second/tool"), b"not actually executed").unwrap();
    fs::set_permissions(
        root.path().join("second/tool"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert_eq!(resolver.resolve("bin:tool:fallback").unwrap(), None);
    symlink("missing", root.path().join("lib/dangling")).unwrap();
    assert_eq!(
        resolver.resolve("path:lib/dangling:fallback").unwrap(),
        None
    );
    assert_eq!(resolver.resolve("path:lib:fallback").unwrap(), None);
    assert_eq!(
        resolver.resolve("path:lib/not*:fallback").unwrap(),
        Some("fallback".into())
    );
    fs::write(root.path().join("lib/libexample.2.dylib"), b"library").unwrap();
    assert_eq!(resolver.resolve("lib:libexample.2:fallback").unwrap(), None);
    assert_eq!(
        resolver.resolve("lib:libexample.3:fallback").unwrap(),
        Some("fallback".into())
    );
    assert_eq!(
        resolver.resolve("port:fallback").unwrap(),
        Some("fallback".into())
    );
}
fn register(writer: &mut Writer, name: &str, actual: Option<&Path>) {
    let mut tx = writer.transaction().unwrap();
    let id = tx
        .register(&NewImage {
            name: name.into(),
            version: "1".into(),
            location: format!("/images/{name}.tbz2"),
            files: actual
                .map(|_| {
                    vec![ImageFile {
                        path: "/payload".into(),
                        binary: false,
                    }]
                })
                .unwrap_or_default(),
            ..NewImage::default()
        })
        .unwrap();
    let files = actual
        .map(|path| {
            vec![ActivationFile {
                path: "/payload".into(),
                actual_path: path.to_str().unwrap().into(),
            }]
        })
        .unwrap_or_default();
    tx.activate(id, &files).unwrap();
    tx.commit().unwrap();
}
#[test]
fn registry_provider_and_active_fallback_follow_upstream_priority() {
    let root = tempfile::tempdir().unwrap();
    let database = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("lib")).unwrap();
    let path = root.path().join("lib/actual");
    fs::write(&path, b"provided").unwrap();
    let path = path.canonicalize().unwrap();
    let dbpath = database.path().join("registry.db");
    let mut writer = Writer::create(&dbpath).unwrap();
    register(&mut writer, "actual-provider", Some(&path));
    let registry = Registry::open(Some(&dbpath)).unwrap().unwrap();
    let resolver = Resolver::new(Some(&registry), search(root.path())).unwrap();
    assert_eq!(
        resolver.resolve("path:lib/actual:fallback").unwrap(),
        Some("actual-provider".into())
    );
    drop(resolver);
    drop(registry);
    register(&mut writer, "fallback", None);
    let registry = Registry::open(Some(&dbpath)).unwrap().unwrap();
    let resolver = Resolver::new(Some(&registry), search(root.path())).unwrap();
    assert_eq!(
        resolver.resolve("path:lib/actual:FALLBACK").unwrap(),
        Some("FALLBACK".into())
    );
}
#[test]
fn library_environment_paths_preserve_upstream_order_and_path_list_semantics() {
    let root = tempfile::tempdir().unwrap();
    let library = root.path().join("literal:directory");
    fs::create_dir(&library).unwrap();
    fs::write(library.join("libunique.1.dylib"), b"library").unwrap();
    let mut context = search(root.path());
    context
        .environment
        .insert("DYLD_LIBRARY_PATH".into(), library.to_str().unwrap().into());
    assert_eq!(
        Resolver::new(None, context)
            .unwrap()
            .resolve("lib:libunique.1:fallback")
            .unwrap(),
        None
    );
    let mut context = search(root.path());
    context.platform = "freebsd".into();
    context
        .environment
        .insert("DYLD_LIBRARY_PATH".into(), library.to_str().unwrap().into());
    fs::write(library.join("libunique.so.1"), b"library").unwrap();
    assert_eq!(
        Resolver::new(None, context)
            .unwrap()
            .resolve("lib:libunique.1:fallback")
            .unwrap(),
        None
    );
}

#[test]
fn runtime_registration_dependencies_use_live_lists_and_omit_system_providers() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("system-provider"), b"present").unwrap();
    let resolver = Resolver::new(None, search(root.path())).unwrap();
    let mut session = port_rs::portfile::Session::open(
        "name app; version 1; depends_lib port:first path:system-provider:unneeded; depends_run port:FIRST port:second; depends_build port:build-only",
        &BTreeMap::new(), &port_rs::spec::PortSpec::named("app"), port_rs::portfile::PortGroups::default()).unwrap();
    assert_eq!(
        resolver.runtime_dependencies(&mut session).unwrap(),
        ["first", "second"]
    );
    session
        .invoke("set", &["depends_run", "port:third"])
        .unwrap();
    assert_eq!(
        resolver.runtime_dependencies(&mut session).unwrap(),
        ["first", "third"]
    );
}

#[test]
fn provider_evidence_distinguishes_missing_fallback_from_system_and_active_ports() {
    use port_rs::dependencies::provider::Resolution;
    let root = tempfile::tempdir().unwrap();
    let resolver = Resolver::new(None, search(root.path())).unwrap();
    let missing = resolver.probe("path:provided:fallback").unwrap();
    assert_eq!(missing, Resolution::Missing("fallback".into()));
    assert!(!missing.is_satisfied());
    fs::write(root.path().join("provided"), b"system").unwrap();
    let present = resolver.probe("path:provided:fallback").unwrap();
    assert_eq!(present, Resolution::System(root.path().join("provided")));
    assert!(present.is_satisfied());
    let database = tempfile::tempdir().unwrap();
    let dbpath = database.path().join("registry.db");
    let mut writer = Writer::create(&dbpath).unwrap();
    register(&mut writer, "fallback", None);
    let registry = Registry::open(Some(&dbpath)).unwrap().unwrap();
    let resolver = Resolver::new(Some(&registry), search(root.path())).unwrap();
    let active = resolver.probe("path:provided:fallback").unwrap();
    assert_eq!(active, Resolution::ActivePort("fallback".into()));
    assert!(active.is_satisfied());
}

#[test]
fn planning_skips_satisfied_providers_but_preserves_explicit_requests_and_missing_fallbacks() {
    use port_rs::{index::Index, install, spec::PortSpec};
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("provided"), b"system").unwrap();
    let resolver = Resolver::new(None, search(root.path())).unwrap();
    let index_text = |fallback: bool| {
        let app = "name app version 1 revision 0 depends_run path:provided:fallback";
        let mut text = format!("app {}\n{app}\n", app.len() + 1);
        if fallback {
            let body = "name fallback version 1 revision 0";
            text.push_str(&format!("fallback {}\n{body}\n", body.len() + 1));
        }
        Index::parse(&text).unwrap()
    };
    let index = index_text(false);
    // A satisfied provider doesn't even require the fallback in the index.
    let plan =
        install::plan_with_providers(&index, &[PortSpec::named("app")], &[], &resolver).unwrap();
    assert_eq!(plan.len(), 1);
    assert!(plan[0].provider_fallbacks.is_empty());
    let index = index_text(true);
    let plan = install::plan_with_providers(
        &index,
        &[PortSpec::named("app"), PortSpec::named("fallback")],
        &[],
        &resolver,
    )
    .unwrap();
    assert_eq!(
        plan.iter()
            .map(|item| item.port.name.as_str())
            .collect::<Vec<_>>(),
        ["fallback", "app"]
    );
    fs::remove_file(root.path().join("provided")).unwrap();
    let plan =
        install::plan_with_providers(&index, &[PortSpec::named("app")], &[], &resolver).unwrap();
    assert_eq!(
        plan.iter()
            .map(|item| item.port.name.as_str())
            .collect::<Vec<_>>(),
        ["fallback", "app"]
    );
    assert!(plan.iter().all(|item| item.provider_fallbacks.is_empty()));
}

#[test]
fn evaluated_planning_avoids_satisfied_fallback_sources_and_preserves_explicit_inputs() {
    use port_rs::{index::Index, install, portfile, spec::PortSpec};
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("provided"), b"system").unwrap();
    let resolver = Resolver::new(None, search(root.path())).unwrap();
    let mut text = String::new();
    for name in ["app", "fallback"] {
        let body = format!("name {name} version 1 revision 0");
        text.push_str(&format!("{name} {}\n{body}\n", body.len() + 1));
    }
    let index = Index::parse(&text).unwrap();
    let mut spec = PortSpec::named("app");
    spec.parse_modifier("+ssl").unwrap();
    for explicit in [false, true] {
        let mut specs = vec![spec.clone()];
        if explicit {
            let mut fallback = PortSpec::named("fallback");
            fallback.parse_modifier("-ssl").unwrap();
            specs.push(fallback);
        }
        let mut loaded = Vec::new();
        let plan =
            install::plan_evaluated_with_providers(&index, &specs, &[], &resolver, |port, spec| {
                loaded.push(port.name.clone());
                let dependency = if port.name == "app" {
                    "depends_run path:provided:fallback"
                } else {
                    ""
                };
                portfile::evaluate(
                    &format!(
                        "name {}; version 1; revision 0; epoch 0; variant ssl {{}}; {dependency}",
                        port.name
                    ),
                    &BTreeMap::new(),
                    spec,
                )
            })
            .unwrap();
        let items: Vec<_> = plan.items().collect();
        assert!(items.iter().all(|item| item.provider_fallbacks.is_empty()));
        if explicit {
            assert_eq!(loaded, ["app", "fallback"]);
            assert_eq!(
                items
                    .iter()
                    .map(|item| item.port.name.as_str())
                    .collect::<Vec<_>>(),
                ["fallback", "app"]
            );
            assert_eq!(items[0].variants, "");
            assert_eq!(items[1].variants, "+ssl");
        } else {
            assert_eq!(loaded, ["app"]);
            assert_eq!(items.len(), 1);
        }
    }
    fs::remove_file(root.path().join("provided")).unwrap();
    let mut loaded = Vec::new();
    let plan =
        install::plan_evaluated_with_providers(&index, &[spec], &[], &resolver, |port, spec| {
            loaded.push(port.name.clone());
            let dependency = if port.name == "app" {
                "depends_run path:provided:fallback"
            } else {
                ""
            };
            portfile::evaluate(
                &format!(
                    "name {}; version 1; revision 0; epoch 0; variant ssl {{}}; {dependency}",
                    port.name
                ),
                &BTreeMap::new(),
                spec,
            )
        })
        .unwrap();
    assert_eq!(loaded, ["app", "fallback"]);
    assert!(
        plan.items()
            .all(|item| item.provider_fallbacks.is_empty() && item.variants == "+ssl")
    );
}

#[test]
fn evaluated_closure_and_final_graph_share_the_same_provider_decisions() {
    use port_rs::{index::Index, install, portfile, spec::PortSpec};
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("provided"), b"system").unwrap();
    let resolver = Resolver::new(None, search(root.path())).unwrap();
    let mut text = String::new();
    for name in ["first", "second"] {
        let body = format!("name {name} version 1 revision 0");
        text.push_str(&format!("{name} {}\n{body}\n", body.len() + 1));
    }
    let index = Index::parse(&text).unwrap();
    let plan = install::plan_evaluated_with_providers(&index, &[PortSpec::named("first"), PortSpec::named("second")], &[], &resolver, |port, spec| {
        if port.name == "second" { fs::remove_file(root.path().join("provided")).unwrap(); }
        portfile::evaluate(&format!("name {}; version 1; revision 0; epoch 0; depends_run path:provided:absent-from-index", port.name), &BTreeMap::new(), spec)
    }).unwrap();
    assert_eq!(plan.items().count(), 2);
    assert!(
        !resolver
            .probe("path:provided:absent-from-index")
            .unwrap()
            .is_satisfied()
    );
}

#[test]
fn session_search_reads_live_environment_without_host_fallback() {
    use port_rs::{
        portfile::{PortGroups, Session},
        spec::PortSpec,
    };
    let root = tempfile::tempdir().unwrap();
    let mut session = Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    for (key, value) in [
        ("prefix", root.path().to_str().unwrap()),
        ("frameworks_dir", "/custom/frameworks"),
        ("os.platform", "darwin"),
        ("env(PATH)", root.path().to_str().unwrap()),
        ("env(DYLD_LIBRARY_PATH)", "/one:/two"),
        ("env(DYLD_FRAMEWORK_PATH)", ""),
    ] {
        session.invoke("set", &[key, value]).unwrap();
    }
    let search = Search::from_session(&mut session).unwrap();
    assert_eq!(search.prefix, root.path());
    assert_eq!(search.frameworks_dir, Path::new("/custom/frameworks"));
    assert_eq!(search.platform, "darwin");
    assert_eq!(search.environment["DYLD_LIBRARY_PATH"], "/one:/two");
    assert_eq!(search.environment["DYLD_FRAMEWORK_PATH"], "");
    let tool = root.path().join("tool");
    fs::write(&tool, b"not executed").unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        Resolver::new(None, search)
            .unwrap()
            .resolve("bin:tool:fallback")
            .unwrap(),
        None
    );
    session.invoke("unset", &["env(PATH)"]).unwrap();
    let search = Search::from_session(&mut session).unwrap();
    assert!(!search.environment.contains_key("PATH"));
    let resolver = Resolver::new(None, search).unwrap();
    assert!(
        resolver
            .resolve("bin:tool:fallback")
            .unwrap_err()
            .to_string()
            .contains("PATH is required")
    );
    assert_eq!(
        resolver.resolve("port:required").unwrap(),
        Some("required".into())
    );
}

#[test]
fn session_planner_uses_each_ports_context_and_retains_live_procedures() {
    use port_rs::{
        index::Index,
        install,
        portfile::{PortGroups, Session},
        spec::PortSpec,
    };
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("present")).unwrap();
    fs::write(root.path().join("present/provider"), b"provided").unwrap();
    let mut text = String::new();
    for name in ["first", "second", "fallback"] {
        let body = format!("name {name} version 1 revision 0");
        text.push_str(&format!("{name} {}\n{body}\n", body.len() + 1));
    }
    let index = Index::parse(&text).unwrap();
    let mut loaded = Vec::new();
    let mut result = install::plan_sessions(&index, &[PortSpec::named("first"), PortSpec::named("second")], None, |port, spec| {
        loaded.push(port.name.clone());
        let prefix = root.path().join(if port.name == "first" { "present" } else { "absent" });
        let variables = BTreeMap::from([
            ("prefix".into(), prefix.to_str().unwrap().into()),
            ("frameworks_dir".into(), root.path().to_str().unwrap().into()),
            ("os.platform".into(), "darwin".into()),
        ]);
        let dependency = if port.name == "fallback" { "" } else { "depends_run path:provider:fallback" };
        Session::open(&format!("name {}; version 1; revision 0; {dependency}; proc retained {{}} {{return ready}}", port.name), &variables, spec, PortGroups::default())
    }).unwrap();
    assert_eq!(loaded, ["first", "second", "fallback"]);
    let items: Vec<_> = result.plan.items().collect();
    assert_eq!(
        items
            .iter()
            .map(|i| i.port.name.as_str())
            .collect::<Vec<_>>(),
        ["first", "fallback", "second"]
    );
    assert!(items.iter().all(|i| i.provider_fallbacks.is_empty()));
    assert_eq!(
        result
            .sessions
            .get_mut("first")
            .unwrap()
            .invoke("retained", &[])
            .unwrap(),
        "ready"
    );
}

#[test]
fn session_planner_rejects_shared_metadata_with_conflicting_search_contexts() {
    use port_rs::{
        index::Index,
        install,
        portfile::{PortGroups, Session},
        spec::PortSpec,
    };
    let mut text = String::new();
    for name in ["first", "second", "shared"] {
        let body = format!("name {name} version 1 revision 0");
        text.push_str(&format!("{name} {}\n{body}\n", body.len() + 1));
    }
    let index = Index::parse(&text).unwrap();
    let mut first = PortSpec::named("first");
    first.parse_modifier("+ssl").unwrap();
    let second = PortSpec::named("second");
    let result = install::plan_sessions(&index, &[first, second], None, |port, spec| {
        let variables = BTreeMap::from([
            (
                "prefix".into(),
                if spec.variants.is_empty() {
                    "/one"
                } else {
                    "/two"
                }
                .into(),
            ),
            ("frameworks_dir".into(), "/frameworks".into()),
            ("os.platform".into(), "darwin".into()),
        ]);
        let dependency = if port.name == "shared" {
            ""
        } else {
            "depends_run port:shared"
        };
        Session::open(
            &format!("name {}; version 1; revision 0; {dependency}", port.name),
            &variables,
            spec,
            PortGroups::default(),
        )
    });
    let error = match result {
        Ok(_) => panic!("conflicting search contexts accepted"),
        Err(error) => format!("{error:#}"),
    };
    assert!(
        error.contains("conflicting provider search contexts"),
        "{error}"
    );
}
