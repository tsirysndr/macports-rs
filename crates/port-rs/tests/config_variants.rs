use port_rs::{config::Variants, spec::PortSpec};

#[test]
fn first_file_choice_wins_but_explicit_requests_override_it() {
    let choices = Variants::parse(
        "# comment\n+ssl -docs\n-ssl +docs\n+with.dot +with-dash +a+b\n",
        "variants.conf",
    );
    assert!(choices.warnings.is_empty());
    assert!(choices.choices["ssl"]);
    assert!(!choices.choices["docs"]);
    assert!(choices.choices["with.dot"]);
    assert!(choices.choices["with-dash"]);
    assert!(choices.choices["a+b"]);
    let mut spec = PortSpec::named("app");
    spec.parse_modifier("-ssl+docs").unwrap();
    choices.apply(&mut spec);
    assert!(!spec.variants["ssl"]);
    assert!(spec.variants["docs"]);
    assert!(spec.variants["with.dot"]);
}
#[test]
fn invalid_tokens_warn_without_discarding_valid_choices() {
    let choices = Variants::parse("  # comment\n+ssl  bad + -bad/name # inline\n", "custom");
    assert_eq!(choices.choices.len(), 1);
    assert!(choices.choices["ssl"]);
    assert_eq!(choices.warnings.len(), 6);
    assert!(choices.warnings[0].contains("syntax ''"));
    assert!(
        choices
            .warnings
            .iter()
            .any(|w| w.contains("syntax '-bad/name'"))
    );
}
#[test]
fn missing_files_are_ignored_and_io_errors_are_reported() {
    let root = tempfile::tempdir().unwrap();
    assert!(Variants::read(None).unwrap().choices.is_empty());
    assert!(
        Variants::read(Some(&root.path().join("missing")))
            .unwrap()
            .choices
            .is_empty()
    );
    assert!(Variants::read(Some(root.path())).is_err());
    let path = root.path().join("variants.conf");
    std::fs::write(&path, "+ssl\n").unwrap();
    assert!(Variants::read(Some(&path)).unwrap().choices["ssl"]);
}
