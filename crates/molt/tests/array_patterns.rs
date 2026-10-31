use molt::Interp;
use std::collections::BTreeMap;

#[test]
fn queries_match_keys_and_keep_values_literal() {
    let mut interp = Interp::new();
    interp.eval(r"array set a {build.cc clang build.cxx clang++ other build.cc {literal*} {$x [error nope]} é one}").unwrap();
    let entries = interp
        .eval("array get a build.*")
        .unwrap()
        .as_list()
        .unwrap();
    let result: BTreeMap<_, _> = entries
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| (p[0].to_string(), p[1].to_string()))
        .collect();
    assert_eq!(
        result,
        BTreeMap::from([
            ("build.cc".into(), "clang".into()),
            ("build.cxx".into(), "clang++".into())
        ])
    );
    for command in [
        "array names a build.c?",
        "array names a -glob build.c?",
        "array names a -g build.c?",
        "array names a -exact build.cc",
        "array names a -e build.cc",
    ] {
        assert_eq!(interp.eval(command).unwrap().as_str(), "build.cc");
    }
    assert_eq!(
        interp.eval(r"array names a {literal\*}").unwrap().as_str(),
        "literal*"
    );
    assert_eq!(interp.eval("array names a ?").unwrap().as_str(), "é");
    assert_eq!(
        interp
            .eval("array names a -exact build.*")
            .unwrap()
            .as_str(),
        ""
    );
    assert_eq!(
        interp.eval("array get a other").unwrap().as_str(),
        "other build.cc"
    );
    assert_eq!(
        interp
            .eval("array get a literal*")
            .unwrap()
            .as_list()
            .unwrap()[1]
            .as_str(),
        "$x [error nope]"
    );
}
#[test]
fn unset_patterns_follow_aliases_and_preserve_empty_arrays() {
    let mut interp = Interp::new();
    interp.eval("array set a {keep 1 drop.a 2 drop.b 3}; proc remove {} {upvar #0 a alias; array unset alias drop.*}; remove").unwrap();
    assert_eq!(interp.eval("array get a").unwrap().as_str(), "keep 1");
    interp.eval("array unset a *").unwrap();
    assert_eq!(interp.eval("array exists a").unwrap().as_str(), "1");
    assert_eq!(interp.eval("array size a").unwrap().as_str(), "0");
    interp.eval("array unset a").unwrap();
    assert_eq!(interp.eval("array exists a").unwrap().as_str(), "0");
}
#[test]
fn missing_arrays_scalars_and_bad_options_follow_query_semantics() {
    let mut interp = Interp::new();
    interp.eval("set scalar unchanged").unwrap();
    for name in ["missing", "scalar"] {
        for action in ["names", "get", "unset"] {
            assert_eq!(
                interp
                    .eval(&format!("array {} {} *", action, name))
                    .unwrap()
                    .as_str(),
                ""
            );
        }
    }
    assert_eq!(interp.eval("set scalar").unwrap().as_str(), "unchanged");
    for script in [
        "array names",
        "array get a x extra",
        "array unset a x extra",
        "array names missing -wrong x",
        "array names missing -regexp x",
    ] {
        assert!(interp.eval(script).is_err(), "{}", script);
    }
}
