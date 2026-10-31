use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::collections::BTreeMap;
fn session() -> Session {
    let mut s = Session::open(
        "name app; version 1; set calls {}",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    s.enable_system(&BTreeMap::new(), |_| {
        panic!("configure checks need no subprocess")
    })
    .unwrap();
    s
}
#[test]
fn finish_dispatches_enabled_checks_in_order_and_honors_replacements() {
    let root = tempfile::tempdir().unwrap();
    let mut s = session();
    s.invoke("set", &["configure.dir", root.path().to_str().unwrap()])
        .unwrap();
    for name in [
        "check_implicit_function_declarations",
        "check_implicit_int",
        "check_incompatible_function_pointer_types",
    ] {
        s.invoke(
            "proc",
            &[
                &format!("portconfigure::{name}"),
                "",
                &format!("global calls; lappend calls {name}"),
            ],
        )
        .unwrap();
    }
    s.invoke("portconfigure::configure_finish", &["ignored"])
        .unwrap();
    assert_eq!(
        s.scalar("calls").unwrap(),
        "check_implicit_function_declarations check_implicit_int check_incompatible_function_pointer_types"
    );
    s.invoke("set", &["calls", ""]).unwrap();
    s.invoke("configure.checks.implicit_int", &["no"]).unwrap();
    s.invoke("portconfigure::configure_finish", &[]).unwrap();
    assert_eq!(
        s.scalar("calls").unwrap(),
        "check_implicit_function_declarations check_incompatible_function_pointer_types"
    );
}
#[test]
fn missing_configure_directory_skips_checks_and_metadata_cannot_run_them() {
    let root = tempfile::tempdir().unwrap();
    let mut s = session();
    s.invoke(
        "set",
        &[
            "configure.dir",
            root.path().join("missing").to_str().unwrap(),
        ],
    )
    .unwrap();
    s.invoke("portconfigure::configure_finish", &[]).unwrap();
    let mut s = Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    assert!(s.invoke("portconfigure::configure_finish", &[]).is_err());
}
#[test]
fn native_finish_scans_logs_with_explicit_whitelist_without_shells() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("config.log"), "warning: implicit declaration of function 'missing' [-Wimplicit-function-declaration]\nwarning: implicit int [-Wimplicit-int]\n").unwrap();
    let mut s = session();
    for key in ["configure.dir", "workpath"] {
        s.invoke("set", &[key, root.path().to_str().unwrap()])
            .unwrap();
    }
    s.invoke(
        "configure.checks.implicit_function_declaration.whitelist",
        &["missing"],
    )
    .unwrap();
    assert_eq!(
        s.invoke("portconfigure::configure_finish", &[]).unwrap(),
        ""
    );
}
