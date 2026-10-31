use molt::Interp;

#[test]
fn lmap_collects_results_and_honours_continue_and_break() {
    let mut interp = Interp::new();
    for (script, expected) in [
        ("lmap x {1 2 3} {expr {$x * 2}}", "2 4 6"),
        ("lmap x {1 2 3 4} {if {$x == 2} continue; set x}", "1 3 4"),
        ("lmap x {1 2 3 4} {if {$x == 3} break; set x}", "1 2"),
        ("lmap {a b} {1 2 3} {list $b $a}", "{2 1} {{} 3}"),
        ("lmap x {} {set x}", ""),
    ] {
        assert_eq!(interp.eval(script).unwrap().as_str(), expected, "{script}");
    }
    assert!(interp.eval("lmap x {1} {error boom}").is_err());
}

#[test]
fn apply_runs_lambdas_with_defaults_args_and_local_scope() {
    let mut interp = Interp::new();
    for (script, expected) in [
        ("apply {{a b} {expr {$a + $b}}} 2 3", "5"),
        ("apply {{a {b 10}} {expr {$a + $b}}} 1", "11"),
        ("apply {args {llength $args}} x y z", "3"),
        ("set v outer; apply {{} {set v inner}}; set v", "outer"),
        ("apply {{x} {return [expr {$x * 3}]}} 4", "12"),
        (
            "set f {{c} {lmap i $c {expr {$i + 1}}}}; apply $f {1 2}",
            "2 3",
        ),
    ] {
        assert_eq!(interp.eval(script).unwrap().as_str(), expected, "{script}");
    }
    for script in [
        "apply {{a} {set a}}",
        "apply {x}",
        "apply {{a} {set a}} 1 2",
    ] {
        assert!(interp.eval(script).is_err(), "{}", script);
    }
}

#[test]
fn return_in_a_sourced_script_ends_only_that_script() {
    let directory = std::env::temp_dir().join(format!("molt-source-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let file = directory.join("early.tcl");
    std::fs::write(&file, "set x early\nreturn sourced\nset x late\n").unwrap();
    let mut interp = Interp::new();
    let path = file.to_str().unwrap();
    assert_eq!(
        interp
            .eval(&format!("proc p {{}} {{source {path}; return after}}; p"))
            .unwrap()
            .as_str(),
        "after"
    );
    assert_eq!(
        interp.eval(&format!("source {path}")).unwrap().as_str(),
        "sourced"
    );
    assert_eq!(interp.eval("set x").unwrap().as_str(), "early");
    std::fs::remove_dir_all(directory).unwrap();
}
