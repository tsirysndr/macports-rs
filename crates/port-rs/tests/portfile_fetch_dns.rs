use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::collections::BTreeMap;
#[test]
fn diagnostic_warns_only_on_positive_result_and_propagates_callback_errors() {
    let mut s = Session::open(
        r#"
        name app; version 1; set warnings {}; set checks 0
        proc check_broken_dns {} {global checks; incr checks; expr {$checks == 1}}
        proc ui_warn message {global warnings; lappend warnings $message}
    "#,
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    s.enable_system(&BTreeMap::new(), |_| panic!("no subprocess expected"))
        .unwrap();
    s.invoke("portfetch::check_dns", &[]).unwrap();
    let warning = s.scalar("warnings").unwrap();
    assert!(warning.contains("checksum mismatches"));
    s.invoke("portfetch::check_dns", &[]).unwrap();
    assert_eq!(s.scalar("warnings").unwrap(), warning);
    s.invoke("proc", &["check_broken_dns", "", "error diagnostic-failed"])
        .unwrap();
    assert!(
        s.invoke("portfetch::check_dns", &[])
            .unwrap_err()
            .to_string()
            .contains("diagnostic-failed")
    );
}
#[test]
fn metadata_sessions_cannot_trigger_dns_queries() {
    for command in ["check_broken_dns", "portfetch::check_dns"] {
        let mut s = Session::open(
            "name app; version 1",
            &BTreeMap::new(),
            &PortSpec::named("app"),
            PortGroups::default(),
        )
        .unwrap();
        assert!(s.invoke(command, &[]).is_err());
    }
}
