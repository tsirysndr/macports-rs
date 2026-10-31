use molt::Interp;

#[test]
fn namespace_values_persist_and_nested_relative_names_are_isolated() {
    let mut interp = Interp::new();
    interp.eval("set value root; namespace eval ::one {variable value first; array set options {key one}; namespace eval child {set value nested}}; namespace eval ::two {variable value second}").unwrap();
    assert_eq!(interp.eval("set value").unwrap().as_str(), "root");
    assert_eq!(interp.eval("set ::one::value").unwrap().as_str(), "first");
    assert_eq!(
        interp
            .eval("namespace eval one {set child::value}")
            .unwrap()
            .as_str(),
        "nested"
    );
    assert_eq!(
        interp
            .eval("namespace eval one {namespace current}")
            .unwrap()
            .as_str(),
        "::one"
    );
    assert_eq!(
        interp
            .eval("namespace eval two {set value}")
            .unwrap()
            .as_str(),
        "second"
    );
    assert_eq!(
        interp.eval("array get ::one::options").unwrap().as_str(),
        "key one"
    );
    assert_eq!(interp.eval("namespace current").unwrap().as_str(), "::");
}
#[test]
fn namespace_links_and_procedures_keep_the_defining_namespace() {
    let mut interp = Interp::new();
    interp.eval("set os.major 24; namespace eval ::one {namespace upvar :: os.major major; variable count 0; proc bump {} {variable count; incr count}; proc call {} {bump}; set observed $major; incr major}; namespace eval ::two {proc bump {} {return other}}").unwrap();
    assert_eq!(interp.eval("set os.major").unwrap().as_str(), "25");
    assert_eq!(interp.eval("set one::observed").unwrap().as_str(), "24");
    assert_eq!(interp.eval("one::call").unwrap().as_str(), "1");
    assert_eq!(
        interp
            .eval("namespace eval two {::one::call}")
            .unwrap()
            .as_str(),
        "2"
    );
    assert_eq!(interp.eval("set one::count").unwrap().as_str(), "2");
    assert_eq!(interp.eval("two::bump").unwrap().as_str(), "other");
}
#[test]
fn failures_restore_namespace_and_procedure_frames() {
    let mut interp = Interp::new();
    assert!(interp
        .eval("namespace eval broken {error failure}")
        .is_err());
    assert_eq!(interp.scope_level(), 0);
    assert_eq!(interp.namespace(), "::");
    interp
        .eval("namespace eval one {proc needs {argument} {return $argument}}")
        .unwrap();
    assert!(interp.eval("one::needs").is_err());
    assert_eq!(interp.scope_level(), 0);
    assert_eq!(interp.namespace(), "::");
    assert_eq!(interp.eval("one::needs good").unwrap().as_str(), "good");
}
#[test]
fn archive_site_namespace_can_link_platform_variables_and_filter_mirrors() {
    let mut interp = Interp::new();
    interp.eval(r#"
        set os.platform darwin; set os.major 24
        namespace eval ::portfetch::mirror_sites {
            namespace upvar :: os.platform os.platform
            namespace upvar :: os.major os.major
            variable protocol [expr {${os.platform} eq "darwin" && ${os.major} > 18 ? "https" : "http"}]
            variable sites
            set sites(primary) [lsearch -all -glob -inline -not "${protocol}://packages.example/:nosubdir ://disabled/" {:*}]
        }
    "#).unwrap();
    assert_eq!(
        interp
            .eval("set ::portfetch::mirror_sites::sites(primary)")
            .unwrap()
            .as_str(),
        "https://packages.example/:nosubdir"
    );
    assert!(!interp.var_exists(&"sites".into()));
}

#[test]
fn unsupported_namespace_operations_do_not_silently_succeed() {
    let mut interp = Interp::new();
    for script in [
        "namespace upvar :: a",
        "namespace upvar :: a(key) local",
        "namespace export *",
        "variable a(key) value",
    ] {
        assert!(interp.eval(script).is_err(), "{}", script);
        assert_eq!(interp.namespace(), "::");
    }
}
