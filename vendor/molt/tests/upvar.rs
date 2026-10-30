use molt::Interp;

#[test]
fn scalar_and_array_aliases_follow_callers_and_preserve_unset_links() {
    let mut interp = Interp::new();
    let value = interp
        .eval(
            r#"
        proc change {scalar array} {
            upvar $scalar value $array values
            incr value
            set values(key) yes
            unset value
            set value restored
        }
        set counter 4
        array set data {}
        change counter data
        list $counter $data(key)
    "#,
        )
        .unwrap();
    assert_eq!(value.as_str(), "restored yes");
}

#[test]
fn absolute_relative_same_scope_and_transitive_aliases() {
    let mut interp = Interp::new();
    let value = interp
        .eval(
            r#"
        set original 1
        proc first {} {upvar #0 original a; second a}
        proc second {name} {upvar 1 $name b; upvar 0 b c; incr c}
        first
        set original
    "#,
        )
        .unwrap();
    assert_eq!(value.as_str(), "2");
    assert_eq!(
        interp
            .eval("upvar #0 original alias; incr alias; set original")
            .unwrap()
            .as_str(),
        "3"
    );
}

#[test]
fn invalid_levels_collisions_cycles_and_element_aliases_are_errors() {
    let mut interp = Interp::new();
    for script in [
        "upvar missing alias",
        "upvar #99 x y",
        "upvar -1 x y",
        "upvar 0 x x",
        "set exists 1; upvar #0 x exists",
        "upvar #0 x",
        "upvar #0 x a extra",
        "upvar #0 data(key) alias",
        "upvar #0 x alias(key)",
    ] {
        assert!(interp.eval(script).is_err(), "{}", script);
    }
    interp.eval("upvar #0 x a").unwrap();
    assert!(interp.eval("upvar #0 a x").is_err());
    interp.eval("set a okay").unwrap();
    assert_eq!(interp.eval("set x").unwrap().as_str(), "okay");
}

#[test]
fn repeated_alias_and_retargeting_do_not_clobber_original_variables() {
    let mut interp = Interp::new();
    let value = interp
        .eval(
            r#"
        set a first; set b second
        proc choose {} {
            upvar #0 a chosen
            upvar #0 a chosen
            upvar #0 b chosen
            set chosen updated
        }
        choose
        list $a $b
    "#,
        )
        .unwrap();
    assert_eq!(value.as_str(), "first updated");
}
