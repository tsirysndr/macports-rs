use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
    tcl_list,
};
use std::collections::BTreeMap;

fn session() -> Session {
    Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap()
}

#[test]
fn joins_paths_without_resolving_parent_components_or_expanding_tildes() {
    let mut session = session();
    for (parts, expected) in [
        (
            vec!["/opt//local/", "lib", "", "pkg/"],
            "/opt/local/lib/pkg",
        ),
        (vec!["/old", "/new//", "./../src"], "/new/./../src"),
        (vec!["/base", "~user/src"], "/base/~user/src"),
        (vec!["", ""], ""),
        (
            vec!["old", "//server///share/", "a b"],
            "//server/share/a b",
        ),
        (vec!["///", "a\\b"], "/a\\b"),
    ] {
        let mut args = vec!["join"];
        args.extend(parts);
        assert_eq!(session.invoke("file", &args).unwrap(), expected);
    }
}

#[test]
fn path_parts_preserve_roots_dots_and_unicode() {
    let mut session = session();
    for (path, dirname, tail, extension, rootname) in [
        ("", ".", "", "", ""),
        ("/", "/", "", "", "/"),
        ("//server/", "//server", "", "", "//server/"),
        (
            "///tmp//foo.tar.gz/",
            "/tmp",
            "foo.tar.gz",
            "",
            "///tmp//foo.tar.gz/",
        ),
        (
            "src/élève.tar.gz",
            "src",
            "élève.tar.gz",
            ".gz",
            "src/élève.tar",
        ),
        (".profile", ".", ".profile", ".profile", ""),
        ("a/..", "a", "..", ".", "a/."),
        ("~user", ".", "~user", "", "~user"),
    ] {
        for (operation, expected) in [
            ("dirname", dirname),
            ("tail", tail),
            ("extension", extension),
            ("rootname", rootname),
        ] {
            assert_eq!(
                session.invoke("file", &[operation, path]).unwrap(),
                expected,
                "{operation} {path}"
            );
        }
    }
    let parts = session
        .invoke("file", &["split", "//server///a b/./../~user/"])
        .unwrap();
    assert_eq!(
        tcl_list::parse(&parts).unwrap(),
        ["//server", "a b", ".", "..", "~user"]
    );
    for (path, expected) in [
        ("/a", "absolute"),
        ("~user", "relative"),
        ("C:\\a", "relative"),
        ("", "relative"),
    ] {
        assert_eq!(
            session.invoke("file", &["pathtype", path]).unwrap(),
            expected
        );
    }
    assert_eq!(session.invoke("file", &["separator"]).unwrap(), "/");
}

#[test]
fn portfiles_can_construct_build_paths_and_wrong_arity_is_recoverable() {
    let mut session = Session::open(
        r#"
        name app; version 1
        set work /tmp/work
        option build.dir [file join $work ${name}-${version}]
        proc source_tail {} {return [file tail [option build.dir]]}
    "#,
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    assert_eq!(session.invoke("source_tail", &[]).unwrap(), "app-1");
    for args in [
        vec![],
        vec!["join"],
        vec!["tail"],
        vec!["dirname", "a", "b"],
        vec!["separator", "a", "b"],
    ] {
        assert!(
            session
                .invoke("file", &args)
                .unwrap_err()
                .to_string()
                .contains("wrong # args")
        );
    }
    assert_eq!(session.invoke("file", &["join", "a", "b"]).unwrap(), "a/b");
}

#[test]
fn unavailable_filesystem_operations_cannot_be_hidden_by_catch() {
    let error = Session::open(
        "name app; version 1; catch {file delete /not-touched}",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("file delete"));
}
