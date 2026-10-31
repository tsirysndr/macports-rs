use port_rs::{
    portfile::{PortGroups, Session},
    progress::Progress,
    spec::PortSpec,
};
use std::collections::BTreeMap;
fn session(source: &str) -> Session {
    let mut s = Session::open(
        source,
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    s.enable_system(&BTreeMap::new(), |_| panic!("no subprocess expected"))
        .unwrap();
    s.set_progress(Progress::new(true));
    s
}
#[test]
fn notes_follow_live_global_writes_unsets_and_are_drained() {
    let mut s = session("name app; version 1; notes original");
    s.invoke("portactivate::activate_finish", &[]).unwrap();
    assert_eq!(
        s.take_notifications(),
        vec![("app".into(), "original".into())]
    );
    assert!(s.take_notifications().is_empty());
    s.invoke("set", &["notes", "updated notes"]).unwrap();
    s.invoke("portactivate::activate_finish", &["ignored"])
        .unwrap();
    assert_eq!(
        s.take_notifications(),
        vec![("app".into(), "updated notes".into())]
    );
    s.invoke("unset", &["notes"]).unwrap();
    s.invoke("portactivate::activate_finish", &[]).unwrap();
    assert!(s.take_notifications().is_empty());
}
#[test]
fn autostart_precedes_notifications_and_cleans_its_flag() {
    for status in [0, 1] {
        let mut s = session(&format!(
            r#"
            name app; version 1; notes ready
            set startupitem_autostart yes
            proc eval_targets target {{
                global portstartupitem::autostart_only loaded
                if {{$target ne "load"}} {{error wrong-target}}
                if {{${{portstartupitem::autostart_only}} ne "yes"}} {{error missing-flag}}
                set loaded yes
                return {status}
            }}
        "#
        ));
        assert_eq!(
            s.invoke("portactivate::activate_finish", &[]).unwrap(),
            status.to_string()
        );
        assert_eq!(s.scalar("loaded").unwrap(), "yes");
        assert_eq!(
            s.invoke("info", &["exists", "portstartupitem::autostart_only"])
                .unwrap(),
            "0"
        );
        assert_eq!(s.take_notifications().len(), usize::from(status == 0));
    }
}
#[test]
fn failed_load_does_not_publish_notes_or_leave_autostart_flag() {
    let mut s = session(
        r#"
        name app; version 1; notes ready
        set startupitem_autostart yes
        proc eval_targets args {error load-failed}
    "#,
    );
    assert!(
        s.invoke("portactivate::activate_finish", &[])
            .unwrap_err()
            .to_string()
            .contains("load-failed")
    );
    assert!(s.take_notifications().is_empty());
    assert_eq!(
        s.invoke("info", &["exists", "portstartupitem::autostart_only"])
            .unwrap(),
        "0"
    );
}
#[test]
fn unwritable_prefix_rejects_activation_start() {
    let root = tempfile::tempdir().unwrap();
    let mut s = session("name app; version 1");
    s.invoke(
        "set",
        &[
            "prefix_frozen",
            root.path().join("missing").to_str().unwrap(),
        ],
    )
    .unwrap();
    assert!(
        s.invoke("portactivate::activate_start", &[])
            .unwrap_err()
            .to_string()
            .contains("activate privilege lifecycle")
    );
}
