use port_rs::{portfile, spec::PortSpec, tcl_list};
use std::collections::BTreeMap;

fn quote(input: &str) -> String {
    let variables = BTreeMap::from([("input".into(), input.into())]);
    let result = portfile::evaluate(
        "name app; version 1; options escaped; option escaped [quotemeta $input]",
        &variables,
        &PortSpec::named("app"),
    )
    .unwrap();
    result.port.get("escaped").into()
}

#[test]
fn regex_quoting_escapes_every_non_word_character() {
    assert_eq!(quote(".tar.xz"), r"\.tar\.xz");
    assert_eq!(quote("a-b c/d@e#f"), r"a\-b\ c\/d\@e\#f");
    assert_eq!(quote(r"[](){}.*+?^$|\"), r"\[\]\(\)\{\}\.\*\+\?\^\$\|\\");
    assert_eq!(quote(""), "");
    assert_eq!(quote("a\n\t\0b"), "a\\\n\\\t\\\0b");
}

#[test]
fn regex_quoting_preserves_letters_decimal_digits_and_underscores() {
    assert_eq!(quote("aZ_09é漢٣"), "aZ_09é漢٣");
    assert_eq!(quote("e\u{301}²😀"), "e\\\u{301}\\²\\😀");
}

#[test]
fn quoting_is_literal_and_can_build_livecheck_patterns() {
    let result = portfile::evaluate(
        r#"
        name app; version 1
        use_xz yes
        livecheck.regex ${name}-(\[0-9.\]+)[quotemeta ${extract.suffix}]
        options literal
        option literal [quotemeta {[error injected]; $name}]
    "#,
        &BTreeMap::new(),
        &PortSpec::named("app"),
    )
    .unwrap();
    assert_eq!(
        tcl_list::parse(result.port.get("livecheck.regex")).unwrap(),
        [r"app-([0-9.]+)\.tar\.xz"]
    );
    assert_eq!(result.port.get("literal"), r"\[error\ injected\]\;\ \$name");
}

#[test]
fn quote_requires_exactly_one_argument() {
    for script in ["quotemeta", "quotemeta first second"] {
        assert!(
            portfile::evaluate(
                &format!("name app; version 1; {script}"),
                &BTreeMap::new(),
                &PortSpec::named("app")
            )
            .is_err()
        );
    }
}
