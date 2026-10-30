use port_rs::query::Matcher;

#[test]
fn matches_recorded_tcl_string_match_results_without_a_tcl_runtime() {
    // Recorded with macOS Tcl 8.5.9 string match, including -nocase. Runtime tests only
    // consume this fixture; neither the application nor CI needs Tcl installed.
    let cases: Vec<(String, String, bool, bool)> =
        serde_json::from_str(include_str!("fixtures/glob.json")).unwrap();
    for (pattern, value, sensitive, expected) in cases {
        assert_eq!(
            Matcher::new(&pattern, false, false, sensitive)
                .unwrap()
                .matches(&value),
            expected,
            "pattern={pattern:?}, value={value:?}, case_sensitive={sensitive}"
        );
    }
}

#[test]
fn stars_do_not_cause_recursive_or_exponential_matching() {
    let pattern = format!("{}b", "*a".repeat(2000));
    let matcher = Matcher::new(&pattern, false, false, true).unwrap();
    assert!(!matcher.matches(&"a".repeat(4000)));
    assert!(matcher.matches(&format!("{}b", "a".repeat(4000))));
}

#[test]
fn question_mark_matches_one_unicode_character() {
    let matcher = Matcher::new("?", false, false, true).unwrap();
    assert!(matcher.matches("é"));
    assert!(matcher.matches("🦀"));
    assert!(!matcher.matches("ab"));
}
