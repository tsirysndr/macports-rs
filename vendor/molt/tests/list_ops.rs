use molt::Interp;

#[test]
fn list_indices_handle_end_arithmetic_nested_lists_and_out_of_range_values() {
    let mut interp = Interp::new();
    for (script, expected) in [
        ("lindex {a b c} end", "c"),
        ("lindex {a b c} end-1", "b"),
        ("lindex {a b c} 1+1", "c"),
        ("lindex {a b c} 2-1", "b"),
        ("lindex {a b c} end+1", ""),
        ("lindex {} end", ""),
        ("lindex {a b c} end-99", ""),
        ("lindex {a {b c}} {end end-1}", "b"),
        ("lindex {a b c} -1+2", "b"),
        ("lindex {a b c} 9223372036854775807+1", ""),
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
fn ranges_are_inclusive_clamped_and_keep_nested_elements() {
    let mut interp = Interp::new();
    for (script, expected) in [
        ("lrange {a b c d} end-2 end", "b c d"),
        ("lrange {a b c d} 1 end-1", "b c"),
        ("lrange {a b} -10 99", "a b"),
        ("lrange {a b} 1 0", ""),
        ("lrange {a b} 0 -1", ""),
        ("lrange {} 0 end", ""),
        ("lrange {a {two words} c} 1 1", "{two words}"),
        ("lrange {a b c} 0x0 0x2-1", "a b"),
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
fn splitting_uses_a_character_set_and_preserves_empty_elements() {
    let mut interp = Interp::new();
    for (script, expected) in [
        ("split {a::b:} :", "a {} b {}"),
        ("split {:a} :", "{} a"),
        ("split {} :", ""),
        ("split {a,b;c} {,;}", "a b c"),
        ("split { aé } {}", "{ } a é { }"),
        ("split {a  b}", "a {} b"),
        ("split {aébéc} é", "a b c"),
        ("split {a\0b} \0", "a b"),
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
fn invalid_indices_and_argument_counts_report_errors_without_evaluation() {
    let mut interp = Interp::new();
    for script in [
        "lrange",
        "split",
        "split a b c",
        "lrange {a} first end",
        "lindex {a} end*2",
        "lrange {a} 0 1+2+3",
        "lrange \\{ 0 end",
    ] {
        assert!(interp.eval(script).is_err(), "{}", script);
    }
    assert!(interp
        .eval("lindex {a} {[error injected]}")
        .unwrap_err()
        .value()
        .as_str()
        .contains("bad index"));
}
