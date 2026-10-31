use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::collections::BTreeMap;
fn session(source: &str) -> Session {
    Session::open(
        source,
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap()
}
fn names(s: &mut Session, target: &str) -> Vec<String> {
    s.target_order(target)
        .unwrap()
        .into_iter()
        .map(|id| s.invoke("ditem_key", &[&id, "name"]).unwrap())
        .collect()
}
#[test]
fn install_order_includes_main_and_does_not_execute_hooks() {
    let mut s = session(
        "name app; version 1; set statusdict preserve; pre-main {error must-not-run}; pre-install {error must-not-run}",
    );
    assert_eq!(
        names(&mut s, "install"),
        [
            "main",
            "archivefetch",
            "fetch",
            "checksum",
            "extract",
            "patch",
            "configure",
            "build",
            "destroot",
            "install"
        ]
        .map(|n| format!("org.macports.{n}"))
    );
    assert_eq!(s.scalar("statusdict").unwrap(), "preserve");
    assert_eq!(names(&mut s, "main"), ["org.macports.main"]);
}
#[test]
fn custom_graph_respects_soft_order_and_rejects_missing_or_cyclic_requirements() {
    let mut s = session(
        r#"
        name app; version 1
        set a [target_new custom.a body]
        target_provides $a a
        set b [target_new custom.b body]
        target_provides $b b
        set c [target_new custom.c body]
        target_provides $c c
        target_requires $c a b
        target_uses $a b
    "#,
    );
    assert_eq!(names(&mut s, "c"), ["custom.b", "custom.a", "custom.c"]);
    let b = s.scalar("b").unwrap();
    s.invoke("target_requires", &[&b, "a"]).unwrap();
    assert!(
        s.target_order("c")
            .unwrap_err()
            .to_string()
            .contains("cyclic")
    );
    s.invoke("ditem_key", &[&b, "requires", "missing"]).unwrap();
    assert!(s.target_order("c").is_err());
    assert!(
        s.target_order("unknown")
            .unwrap_err()
            .to_string()
            .contains("unknown target")
    );
    // Scope cleanup works even after planning errors.
    assert_eq!(names(&mut s, "main"), ["org.macports.main"]);
}
