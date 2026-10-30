use molt::Interp;
#[test]
fn glob_exact_inverse_inline_and_all_preserve_list_order() {
    let mut interp = Interp::new();
    for (script, expected) in [
        ("lsearch {abc bcd abc} a*", "0"),
        ("lsearch -all {abc bcd abc} a*", "0 2"),
        ("lsearch -all -inline -not {abc bcd abc} a*", "bcd"),
        ("lsearch -exact {abc a*} a*", "1"),
        ("lsearch -exact -glob {abc a*} a*", "0"),
        ("lsearch -nocase {É abc} é", "0"),
        ("lsearch -exact -nocase {É abc} é", "0"),
        ("lsearch {abc} absent", "-1"),
        ("lsearch -inline {abc} absent", ""),
        ("lsearch -all {abc} absent", ""),
        ("lsearch -all -inl {a b a} a", "a a"),
        ("lsearch -exact -integer {1 02 3} 2", "1"),
        ("lsearch -exact -real {1.0 2.5} 1", "0"),
        ("lsearch -integer {text} t*", "0"),
        ("lsearch -start end-1 {a b a b} b", "3"),
        ("lsearch -start -2 {a b} a", "0"),
        ("lsearch -start 50 {a b} a", "-1"),
        ("lsearch -exact -integer -start 50 {a b} not-a-number", "-1"),
    ] {
        assert_eq!(
            interp.eval(script).unwrap().as_str(),
            expected,
            "{}",
            script
        );
    }
}
#[test]
fn archive_mirror_filter_retains_available_protocols_in_order() {
    let mut interp = Interp::new();
    let value = interp
        .eval(
            r#"
        set enabled https
        set disabled {}
        lsearch -all -glob -inline -not "
            ${enabled}://packages.example/:nosubdir
            ${disabled}://unavailable.example/:nosubdir
            http://fallback.example/:nosubdir
        " {:*}
    "#,
        )
        .unwrap();
    let list = value.as_list().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].as_str(), "https://packages.example/:nosubdir");
    assert_eq!(list[1].as_str(), "http://fallback.example/:nosubdir");
}
#[test]
fn invalid_options_lists_and_numeric_values_are_errors() {
    let mut interp = Interp::new();
    for script in [
        "lsearch",
        "lsearch -i {} x",
        "lsearch -unknown {} x",
        "lsearch -start {} x",
        "lsearch -start bad {} x",
        "lsearch -exact -integer {text} 2",
        "lsearch -exact -real {1} text",
        "lsearch -regexp {} x",
        r"lsearch \{ x",
    ] {
        assert!(interp.eval(script).is_err(), "{}", script);
    }
}
