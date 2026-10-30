use molt::Interp;

#[test]
fn substitutions_are_ordered_and_replacements_are_not_rescanned() {
    let mut interp = Interp::new();
    for (script, expected) in [
        (
            "string map {abc 1 ab 2 a 3 1 0} 1abcaababcabababc",
            "01321221",
        ),
        (
            "string map {1 0 ab 2 a 3 abc 1} 1abcaababcabababc",
            "02c322c222c",
        ),
        ("string map {a b b c} ab", "bc"),
        ("string map {a first a second} a", "first"),
        ("string map {{} ignored x {}} xxx", ""),
        ("string map {} unchanged", "unchanged"),
        ("string map {a b} {}", ""),
        ("string map {x y} é😀", "é😀"),
    ] {
        assert_eq!(interp.eval(script).unwrap().as_str(), expected, "{script}");
    }
}

#[test]
fn nocase_uses_unicode_characters_without_expanding_them() {
    let mut interp = Interp::new();
    for (script, expected) in [
        ("string map -nocase {ab X a Y} ABa", "XY"),
        ("string map -n {é 🌱 i X} Éİi", "🌱XX"),
        ("string map -nocase {ss X} ßSS", "ßX"),
        ("string map -nocase {é a} éÉe", "aae"),
        ("string map -nocase {😀 yes} 😀", "yes"),
        ("string map {é a} Éé", "Éa"),
    ] {
        assert_eq!(interp.eval(script).unwrap().as_str(), expected, "{script}");
    }
}

#[test]
fn substitutions_preserve_literal_script_syntax_and_nuls() {
    let mut interp = Interp::new();
    let script =
        r#"set name git; string map [list @name@ $name] {system "make @name@"; $later [deferred]}"#;
    assert_eq!(
        interp.eval(script).unwrap().as_str(),
        "system \"make git\"; $later [deferred]"
    );
    assert_eq!(
        interp.eval("string map {\0 X} {a\0b}").unwrap().as_str(),
        "aXb"
    );
}

#[test]
fn invalid_mapping_and_options_fail_even_for_empty_input() {
    let mut interp = Interp::new();
    for (script, expected) in [
        ("string map {a} {}", "char map list unbalanced"),
        ("string map - {} {}", "bad option"),
        ("string map -nocasex {} {}", "bad option"),
        ("string map", "wrong # args"),
        ("string map a b c d", "wrong # args"),
        ("string map \\{ {}", "unmatched open brace"),
    ] {
        let error = interp.eval(script).unwrap_err().value().to_string();
        assert!(error.contains(expected), "{}: {}", script, error);
    }
}
