use molt::{glob::Glob, Interp, Value};

#[test]
fn matches_tcl_patterns_with_unicode_escaping_ranges_and_nocase() {
    let mut interp = Interp::new();
    for (script, expected) in [
        ("string match {git*} git-core", "1"),
        ("string match {git?} git", "0"),
        ("string match -nocase {É?} é😀", "1"),
        ("string match -n {[Z-A]} q", "1"),
        (r"string match {a\*b} {a*b}", "1"),
        ("string match {[!a]} !", "1"),
        ("string match {[!a]} z", "0"),
        ("string match {} {}", "1"),
        ("string match {*} {}", "1"),
        ("string match -literal -literal", "1"),
        ("string match {a?b} {a\0b}", "1"),
        ("string match -nocase ss ß", "0"),
    ] {
        assert_eq!(interp.eval(script).unwrap().as_str(), expected, "{script}");
    }
}
#[test]
fn malformed_classes_follow_tcl_early_match_behavior() {
    for (pattern, value, expected) in [
        ("[ab-]x", "ax", true),
        ("[ab-]x", "a", false),
        ("[ab-]x", "x", true),
        ("[a-", "a", false),
        ("[ab-", "a", true),
        ("[ab-", "b", false),
        ("[a", "a", true),
        ("[]", "]", false),
        ("\\", "\\", false),
    ] {
        assert_eq!(
            Glob::new(pattern, true).matches(value),
            expected,
            "{pattern:?} {value:?}"
        );
    }
}
#[test]
fn patterns_are_literal_data_and_invalid_options_report_errors() {
    let mut interp = Interp::new();
    let literal = "$value [error nope]; text";
    let command = Value::from(vec![
        Value::from("string"),
        Value::from("match"),
        Value::from(literal),
        Value::from(literal),
    ]);
    // Brackets are glob syntax, never command substitution.
    assert_eq!(interp.eval_value(&command).unwrap().as_str(), "0");
    for script in [
        "string match",
        "string match x",
        "string match - x x",
        "string match -bad x x",
        "string match -nocase x x extra",
    ] {
        assert!(interp.eval(script).is_err(), "{}", script);
    }
}
