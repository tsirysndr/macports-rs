use molt::{interp::ScalarChange, ContextID, Exception, Interp, Value};
#[derive(Default)]
struct State {
    writes: Vec<String>,
    unsets: Vec<String>,
}
fn read(_: &mut Interp, _: ContextID, name: &str) -> Result<Option<Value>, Exception> {
    Ok((name == "computed").then(|| Value::from(42)))
}
fn changed(
    interp: &mut Interp,
    id: ContextID,
    name: &str,
    change: ScalarChange,
) -> Result<(), Exception> {
    let state = interp.context::<State>(id);
    match change {
        ScalarChange::Write => state.writes.push(name.into()),
        ScalarChange::Unset => state.unsets.push(name.into()),
    }
    Ok(())
}
#[test]
fn host_hooks_follow_global_aliases_but_ignore_local_shadows() {
    let mut interp = Interp::new();
    let id = interp.save_context(State::default());
    interp.set_global_scalar_hooks(id, read, changed);
    assert_eq!(
        interp
            .eval("set computed placeholder; set computed")
            .unwrap()
            .as_str(),
        "42"
    );
    assert_eq!(
        interp
            .eval("proc local {} {set computed 7; return $computed}; local")
            .unwrap()
            .as_str(),
        "7"
    );
    assert_eq!(
        interp
            .eval("proc alias {} {global computed; set computed 8; return $computed}; alias")
            .unwrap()
            .as_str(),
        "42"
    );
    interp
        .eval("proc remove {} {global computed; unset computed}; remove")
        .unwrap();
    assert_eq!(
        interp
            .context::<State>(id)
            .writes
            .iter()
            .filter(|name| *name == "computed")
            .count(),
        2
    );
    assert!(interp
        .context::<State>(id)
        .unsets
        .contains(&"computed".into()));
}
#[test]
fn parse_errors_do_not_leak_the_recursion_budget() {
    let mut interp = Interp::new();
    interp.set_recursion_limit(10);
    for _ in 0..100 {
        assert!(interp.eval("{").is_err());
    }
    assert_eq!(interp.eval("set result ok").unwrap().as_str(), "ok");
}
#[test]
fn global_substitution_preserves_scope_and_literal_punctuation() {
    let mut interp = Interp::new();
    interp.set_scalar("name", "global".into()).unwrap();
    interp.push_scope();
    interp.set_scalar("name", "local".into()).unwrap();
    assert_eq!(
        interp
            .subst_global("\"$name\"; {[list item]}")
            .unwrap()
            .as_str(),
        "\"global\"; {item}"
    );
    assert_eq!(interp.scalar("name").unwrap().as_str(), "local");
    assert!(interp.subst_global("$missing").is_err());
    assert_eq!(interp.scalar("name").unwrap().as_str(), "local");
    interp.pop_scope();
}

#[test]
fn increment_reports_invalid_values_and_overflow_without_overwriting() {
    let mut interp = Interp::new();
    interp.eval("set value invalid").unwrap();
    assert!(interp.eval("incr value").is_err());
    assert_eq!(interp.scalar("value").unwrap().as_str(), "invalid");
    interp.eval("set value 9223372036854775807").unwrap();
    assert!(interp.eval("incr value").is_err());
    assert_eq!(
        interp.scalar("value").unwrap().as_str(),
        "9223372036854775807"
    );
}
