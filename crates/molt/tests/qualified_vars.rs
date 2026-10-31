use molt::Interp;

#[test]
fn qualified_scalars_bypass_locals_in_scripts_expressions_and_mutators() {
    let mut interp = Interp::new();
    let result = interp
        .eval(
            r#"
        set name root
        set count 3
        proc check {} {
            set name local
            set count 100
            incr ::count
            append ::name -changed
            list $name $::name ${::count} [expr {$::count + $count}]
        }
        check
    "#,
        )
        .unwrap();
    assert_eq!(result.as_str(), "local root-changed 4 104");
    assert_eq!(interp.scalar("name").unwrap().as_str(), "root-changed");
}

#[test]
fn namespace_separators_are_parsed_and_single_colons_remain_literal() {
    let mut interp = Interp::new();
    // Host modules may define qualified globals without evaluating a namespace body.
    interp
        .set_scalar("portutil::autoconf::make_path", "/usr/bin/make".into())
        .unwrap();
    let result = interp.eval(r#"
        set name value
        proc check {} {
            list $::name:suffix $:literal $::portutil::autoconf::make_path ${::portutil::autoconf::make_path}
        }
        check
    "#).unwrap();
    assert_eq!(
        result.as_str(),
        "value:suffix {$:literal} /usr/bin/make /usr/bin/make"
    );
    assert!(interp.eval("set value $::does_not_exist").is_err());
}

#[test]
fn qualified_arrays_and_indices_share_storage_and_unsets_do_not_create_values() {
    let mut interp = Interp::new();
    let result = interp
        .eval(
            r#"
        array set data {key root}
        proc check {} {
            set key key
            set data(key) local
            set ::data(other) created
            set before [list $data(key) $::data($key) [array size ::data]]
            unset ::data(key)
            unset -nocomplain ::missing(key)
            list $before [info exists ::data(key)] [info exists ::missing]
        }
        check
    "#,
        )
        .unwrap();
    assert_eq!(result.as_str(), "{local root 2} 0 0");
    assert_eq!(interp.eval("set data(other)").unwrap().as_str(), "created");
}

#[test]
fn global_imports_namespace_tail_and_upvar_respects_explicit_global_names() {
    let mut interp = Interp::new();
    interp.set_scalar("module::value", "one".into()).unwrap();
    let result = interp
        .eval(
            r#"
        proc modify {} {
            global ::module::value
            append value -two
            upvar 0 ::module::value another
            append another -three
            return $module::value
        }
        modify
    "#,
        )
        .unwrap();
    assert_eq!(result.as_str(), "one-two-three");
    assert_eq!(
        interp.scalar("module::value").unwrap().as_str(),
        "one-two-three"
    );
    assert!(interp
        .eval("proc bad {} {set local 1; upvar 0 local ::escaped}; bad")
        .is_err());
}

#[test]
fn repeated_namespace_separators_and_qualified_unsets_are_canonical() {
    let mut interp = Interp::new();
    interp.set_scalar("module::value", "one".into()).unwrap();
    let result = interp
        .eval(
            r#"
        set :single literal
        set :single::::child nested
        proc check {} {
            set :::::module::::value changed
            list $::::module:::value ${::module::value} $:literal
        }
        check
    "#,
        )
        .unwrap();
    assert_eq!(result.as_str(), "changed changed {$:literal}");
    assert_eq!(interp.scalar(":single::child").unwrap().as_str(), "nested");
    interp.eval("unset ::module::value").unwrap();
    assert!(interp.scalar("module::value").is_err());
    assert_eq!(interp.scalar(":single").unwrap().as_str(), "literal");
}

#[test]
fn root_global_enumeration_excludes_namespace_storage() {
    let mut interp = Interp::new();
    interp.set_scalar("module::value", "nested".into()).unwrap();
    interp.set_scalar("value", "root".into()).unwrap();
    assert!(!interp
        .vars_in_global_scope()
        .iter()
        .any(|name| name.as_str() == "module::value"));
    assert_eq!(
        interp
            .eval("proc check {} {global {*}[info globals]; return $value}; check")
            .unwrap()
            .as_str(),
        "root"
    );
}

#[test]
fn qualified_procedure_parameters_cannot_write_global_storage() {
    let mut interp = Interp::new();
    interp
        .eval("set value original; proc existing {} {return preserved}")
        .unwrap();
    for parameters in ["::value", "module::value", "data(key)"] {
        let script = format!("proc existing {{{}}} {{return replaced}}", parameters);
        assert!(interp.eval(&script).is_err(), "{}", script);
    }
    assert_eq!(interp.eval("existing").unwrap().as_str(), "preserved");
    assert_eq!(interp.scalar("value").unwrap().as_str(), "original");
    assert_eq!(
        interp
            .eval("proc single {a:b} {return ${a:b}}; single allowed")
            .unwrap()
            .as_str(),
        "allowed"
    );
}
