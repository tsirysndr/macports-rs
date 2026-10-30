use molt::{Interp, Value};
#[test]
fn tcl9_radices_signs_and_signed_boundaries() {
    for (text, expected) in [
        ("0xFF", 255),
        ("0Xff", 255),
        ("+0o755", 493),
        ("-0O755", -493),
        ("0b101", 5),
        ("0B101", 5),
        ("0d077", 77),
        ("0D077", 77),
        ("077", 77),
        ("08", 8),
        ("\t -0x8000000000000000 \n", i64::MIN),
        ("-9223372036854775808", i64::MIN),
        ("9223372036854775807", i64::MAX),
    ] {
        assert_eq!(Value::get_int(text).unwrap(), expected, "{}", text);
    }
    for text in [
        "++1",
        "--1",
        "+-1",
        "-+1",
        "0x+1",
        "0o-1",
        "0b2",
        "0o8",
        "0dA",
        "0X",
        "0B",
        "0O",
        "0D",
        "+",
        "-",
        "9223372036854775808",
        "-9223372036854775809",
        "18446744073709551616",
    ] {
        assert!(Value::get_int(text).is_err(), "{}", text);
    }
}
#[test]
fn prefixed_numbers_work_in_expressions_increments_and_list_indices() {
    let mut interp = Interp::new();
    for (script, expected) in [
        ("expr {0o10 + 0B11 + 0d2 + 0Xf}", "28"),
        ("set value -0o10; expr {$value + 0b1}", "-7"),
        ("set value 0O10; incr value 0b10", "10"),
        ("lindex {zero one two three} 0b10", "two"),
        ("expr {08 + 077}", "85"),
        (
            "set value -9223372036854775808; incr value 1",
            "-9223372036854775807",
        ),
    ] {
        assert_eq!(interp.eval(script).unwrap().as_str(), expected, "{script}");
    }
}
#[test]
fn malformed_numeric_literals_return_errors_instead_of_panicking() {
    let mut interp = Interp::new();
    for text in ["0x", "0X", "0b", "0o", "0d", "0b2", "0o8"] {
        assert!(
            interp.eval(&format!("expr {{{text} + 1}}")).is_err(),
            "{}",
            text
        );
        interp.set_scalar("value", text.into()).unwrap();
        assert!(interp.eval("expr {$value + 1}").is_err(), "{}", text);
    }
    interp.set_scalar("value", Value::from(i64::MIN)).unwrap();
    assert!(interp.eval("expr {-$value}").is_err());
    assert!(interp.eval("expr {abs($value)}").is_err());
    assert_eq!(interp.eval("expr {2 + 3}").unwrap().as_str(), "5");
}
