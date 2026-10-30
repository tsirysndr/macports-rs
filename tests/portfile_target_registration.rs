use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
    tcl_list,
};
use std::collections::BTreeMap;
fn open(body: &str) -> Session {
    Session::open(
        &format!("name app; version 1; set events {{}}; {body}"),
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap()
}
fn item(session: &Session, name: &str) -> BTreeMap<String, String> {
    session
        .metadata()
        .dependency_items
        .values()
        .find(|fields| fields.get("name").is_some_and(|value| value == name))
        .unwrap()
        .clone()
}

#[test]
fn custom_target_hooks_and_do_phase_share_the_original_context() {
    let mut session = open(
        r#"
        proc original {phase} {global events; lappend events original:$phase; return 0}
        set custom [target_new org.custom.compile original]
        target_provides $custom custom-compile
        pre-custom-compile {lappend events pre1}
        pre-custom-compile {lappend events pre2}
        custom-compile {lappend events body; do-custom-compile}
        post-custom-compile {lappend events post}
    "#,
    );
    assert_eq!(session.scalar("events").unwrap(), "");
    let target = item(&session, "org.custom.compile");
    assert_eq!(target["provides"], "custom-compile");
    let mut callbacks = tcl_list::parse(&target["pre"]).unwrap();
    callbacks.push(target["procedure"].clone());
    callbacks.extend(tcl_list::parse(&target["post"]).unwrap());
    for callback in callbacks {
        assert_eq!(
            session.invoke(&callback, &["org.custom.compile"]).unwrap(),
            "0"
        );
    }
    assert_eq!(
        session.scalar("events").unwrap(),
        "pre1 pre2 body original:custom-compile post"
    );
}

#[test]
fn existing_phase_commands_keep_their_binding_while_hooks_rebind() {
    let mut session = open(
        r#"
        proc original {phase} {global events; lappend events original:$phase}
        set first [target_new first original]
        set second [target_new second original]
        target_provides $first custom
        target_provides $second custom
        custom {lappend events body}
        pre-custom {lappend events second-before}
    "#,
    );
    let first = item(&session, "first");
    let second = item(&session, "second");
    assert!(first["procedure"].starts_with("proc-first-custom-"));
    assert!(!first.contains_key("pre"));
    assert_eq!(second["procedure"], "original");
    let callback = tcl_list::parse(&second["pre"]).unwrap();
    session.invoke(&callback[0], &["second"]).unwrap();
    session.invoke(&first["procedure"], &["first"]).unwrap();
    assert_eq!(session.scalar("events").unwrap(), "second-before body");
}

#[test]
fn existing_nonphase_commands_are_not_replaced_by_target_provides() {
    let mut session = open(
        r#"
        proc custom {value} {return existing:$value}
        set custom [target_new custom-item missing-original]
        target_provides $custom custom custom
        pre-custom {lappend events hooked}
    "#,
    );
    assert_eq!(
        session.invoke("custom", &["value"]).unwrap(),
        "existing:value"
    );
    let target = item(&session, "custom-item");
    assert_eq!(target["provides"], "custom custom");
    assert_eq!(target["procedure"], "missing-original");
    assert!(!session.metadata().targets.contains_key("custom"));
    assert_eq!(target["pre"], "proc-pre-custom-item-custom-0");
}

#[test]
fn renamed_declarations_preserve_phase_identity_and_original_procedure_capture() {
    let mut session = open(
        r#"
        proc original {phase} {return original:$phase}
        proc replacement {phase} {error wrong-original}
        set handle [target_new original-name original]
        target_provides $handle custom
        ditem_key $handle name changed-name
        ditem_key $handle procedure replacement
        rename custom declaration
        rename pre-custom before-declaration
        declaration {lappend events body}
        before-declaration {lappend events before}
    "#,
    );
    let target = item(&session, "changed-name");
    assert_eq!(target["procedure"], "proc-original-name-custom-0");
    assert_eq!(target["pre"], "proc-pre-original-name-custom-0");
    assert_eq!(session.invoke("do-custom", &[]).unwrap(), "original:custom");
    assert!(session.metadata().targets.contains_key("custom"));
    assert!(!session.metadata().targets.contains_key("declaration"));
    assert_eq!(session.metadata().hooks[0].phase, "pre-custom");
}

#[test]
fn multiple_provided_tokens_capture_the_same_original_implementation() {
    let mut session = open(
        r#"
        proc original {phase} {return $phase}
        set handle [target_new custom original]
        target_provides $handle first second
        first {error replaced}
        second {}
    "#,
    );
    assert_eq!(session.invoke("do-first", &[]).unwrap(), "first");
    assert_eq!(session.invoke("do-second", &[]).unwrap(), "second");
    let target = item(&session, "custom");
    assert_eq!(target["procedure"], "proc-custom-second-0");
    assert_eq!(
        session.invoke(&target["procedure"], &["custom"]).unwrap(),
        "0"
    );
}

#[test]
fn generated_original_calls_and_wrapper_names_treat_metacharacters_as_data() {
    let mut session = open(
        r#"
        set token {custom ;[error injected]}
        set delegate "do-$token"
        proc {original ;[error injected]} {phase} {global events; lappend events $phase}
        set handle [target_new {item ;[error injected]} {original ;[error injected]}]
        target_provides $handle $token
        $token {$delegate}
    "#,
    );
    let target = item(&session, "item ;[error injected]");
    session.invoke(&target["procedure"], &["unused"]).unwrap();
    assert_eq!(
        tcl_list::parse(&session.scalar("events").unwrap()).unwrap(),
        ["custom ;[error injected]"]
    );
}

#[test]
fn builtin_do_phase_uses_original_native_procedure_and_propagates_its_error() {
    let mut session = open(
        r#"
        proc portbuild::build_main {phase} {return original:$phase}
        build {error replaced}
        build {lappend events body}
    "#,
    );
    assert_eq!(session.invoke("do-build", &[]).unwrap(), "original:build");
    session
        .invoke(
            "proc",
            &["portbuild::build_main", "phase", "error original-failed"],
        )
        .unwrap();
    assert!(
        session
            .invoke("do-build", &[])
            .unwrap_err()
            .to_string()
            .contains("original-failed")
    );
    assert!(session.invoke("do-build", &["unexpected"]).is_err());
}
