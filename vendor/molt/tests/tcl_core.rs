use molt::Interp;

fn check(cases: &[(&str, &str)]) {
    let mut interp = Interp::new();
    for (script, expected) in cases {
        match interp.eval(script) {
            Ok(value) => assert_eq!(value.as_str(), *expected, "{script}"),
            Err(error) => panic!("{script}: {}", error.value()),
        }
    }
}

fn fails(scripts: &[&str]) {
    let mut interp = Interp::new();
    for script in scripts {
        assert!(interp.eval(script).is_err(), "{script}");
    }
}

#[test]
fn control_flow_commands() {
    check(&[
        ("switch b {a {set r 1} b {set r 2} default {set r 3}}", "2"),
        ("switch -- z a {set r 1} default {set r 3}", "3"),
        (
            "switch -glob foo.tar {*.zip {set r z} *.tar {set r t}}",
            "t",
        ),
        (
            "switch -regexp abc123 {{^[a-z]+$} {set r w} {\\d+$} {set r n}}",
            "n",
        ),
        ("switch x {x - y {set r xy} z {set r z}}", "xy"),
        ("switch -nocase ABC {abc {set r ok}}", "ok"),
        ("switch q {a {set r 1}}", ""),
        ("eval set e1 [list {hello world}]", "hello world"),
        ("eval {set e2 5}", "5"),
        (
            "proc outer {} {set v 1; inner; return $v}; proc inner {} {uplevel 1 {set v 2}}; outer",
            "2",
        ),
        (
            "set g 0; proc deep {} {uplevel #0 {set g 9}}; deep; set g",
            "9",
        ),
        (
            "set name W; subst {Hello $name [string length abc] \\t}",
            "Hello W 3 \t",
        ),
        ("subst -nocommands {[x] $name}", "[x] W"),
        ("subst -novariables {$name}", "$name"),
        ("concat {a b} { c } {} d", "a b c d"),
    ]);
    fails(&["switch a {b}", "uplevel 99 {set x 1}"]);
}

#[test]
fn list_commands() {
    check(&[
        ("lassign {1 2 3 4} a b", "3 4"),
        ("lassign {1} a b; list $a $b", "1 {}"),
        ("lreplace {a b c d} 1 2 X", "a X d"),
        ("lreplace {a b c} end end", "a b"),
        ("lreplace {a b c} 1 0 X Y", "a X Y b c"),
        ("linsert {a b c} 1 X", "a X b c"),
        ("linsert {a b c} end X", "a b c X"),
        ("linsert {a b c} end-1 X", "a b X c"),
        ("lreverse {1 2 3}", "3 2 1"),
        ("lrepeat 3 a b", "a b a b a b"),
        ("set l {a {b c} d}; lset l 1 0 X", "a {X c} d"),
        ("set l2 {a b}; lset l2 end Z", "a Z"),
        ("lsort {c a b}", "a b c"),
        ("lsort -decreasing {c a b}", "c b a"),
        ("lsort -integer {10 9 100}", "9 10 100"),
        ("lsort -real {1.5 1.25 10}", "1.25 1.5 10"),
        ("lsort -dictionary {a10 a9 A2 b1}", "A2 a9 a10 b1"),
        ("lsort -unique {b a b c a}", "a b c"),
        ("lsort -index 1 {{x 3} {y 1} {z 2}}", "{y 1} {z 2} {x 3}"),
        ("lsort -nocase {B a C}", "a B C"),
        ("proc cmp {a b} {expr {[string length $a] - [string length $b]}}; lsort -command cmp {ccc a bb}", "a bb ccc"),
    ]);
    fails(&["lsort -integer {a b}", "lsort -bogus {a}"]);
}

#[test]
fn string_subcommands() {
    check(&[
        ("string first b abcb", "1"),
        ("string first b abcb 2", "3"),
        ("string first z abc", "-1"),
        ("string last b abcb", "3"),
        ("string index abc 1", "b"),
        ("string index abc end", "c"),
        ("string index abc 5", ""),
        ("string range abcdef 1 3", "bcd"),
        ("string range abcdef 2 end", "cdef"),
        ("string range abcdef end-1 end", "ef"),
        ("string range abc 2 1", ""),
        ("string tolower ABC", "abc"),
        ("string toupper abc", "ABC"),
        ("string toupper abc 1 1", "aBc"),
        ("string totitle hELLO", "Hello"),
        ("string trim {  x  }", "x"),
        ("string trim xxhixx x", "hi"),
        ("string trimleft {  x }", "x "),
        ("string trimright {.so.1.} .", ".so.1"),
        ("string repeat ab 3", "ababab"),
        ("string reverse abc", "cba"),
        ("string replace abcdef 1 2 XY", "aXYdef"),
        ("string replace abc 1 1", "ac"),
        ("string is integer 42", "1"),
        ("string is integer 4.2", "0"),
        ("string is digit {}", "1"),
        ("string is digit -strict {}", "0"),
        ("string is true yes", "1"),
        ("string is false yes", "0"),
        ("string is alpha abc", "1"),
        ("string is space { }", "1"),
        ("string is double 1.5", "1"),
        ("string is alnum -failindex where ab-c; set where", "2"),
    ]);
}

#[test]
fn format_conversions() {
    check(&[
        ("format %s-%d abc 42", "abc-42"),
        ("format %5s| x", "    x|"),
        ("format %-5s| x", "x    |"),
        ("format %05d -42", "-0042"),
        ("format %x 255", "ff"),
        ("format %#X 255", "0XFF"),
        ("format %o 8", "10"),
        ("format %c 65", "A"),
        ("format %.2f 3.14159", "3.14"),
        ("format %e 12345.678", "1.234568e+04"),
        ("format %g 0.0001", "0.0001"),
        ("format %g 100000", "100000"),
        ("format %g 1000000", "1e+06"),
        ("format %+d 5", "+5"),
        ("format %.3s abcdef", "abc"),
        ("format %*d 4 7", "   7"),
        ("format {%2$s %1$s} a b", "b a"),
        ("format 100%%", "100%"),
    ]);
    fails(&["format %d", "format %d abc", "format %y 1"]);
}

#[test]
fn regular_expressions() {
    check(&[
        (
            "regexp {(\\d+)\\.(\\d+)} {v1.23} m a b; list $m $a $b",
            "1.23 1 23",
        ),
        ("regexp -nocase {^ABC} abcdef", "1"),
        ("regexp {^x} abc", "0"),
        ("regexp -inline {(a)(b)?} ac", "a a {}"),
        ("regexp -all -inline {\\d} a1b2c3", "1 2 3"),
        ("regexp -all {o} foo", "2"),
        ("regexp -indices {b+} abbc span; set span", "1 2"),
        ("regexp {a.b} \"a\\nb\"", "1"),
        ("regexp -line {a.b} \"a\\nb\"", "0"),
        ("regexp {\\mword\\M} {a word here}", "1"),
        ("regexp -start 2 {a} abca", "1"),
        ("regexp {[[:digit:]]+} abc123 d; set d", "123"),
        ("regsub {o} foo 0", "f0o"),
        ("regsub -all {o} foo 0", "f00"),
        ("regsub -all {(\\w+)@(\\w+)} {a@b c@d} {\\2@\\1}", "b@a d@c"),
        ("regsub {b+} abbc {[&]}", "a[bb]c"),
        ("regsub {b} abc {\\&}", "a&c"),
        ("regsub -all {x} abc y out", "0"),
        ("regsub -all {b} abcb B out; set out", "aBcB"),
        ("regsub -all {} abc -", "-a-b-c-"),
        ("regsub -nocase {A} abc Z", "Zbc"),
    ]);
    fails(&["regexp {(} x", "regexp -inline {a} a m"]);
}

#[test]
fn glob_matches_files_with_types_tails_and_braces() {
    let root = std::env::temp_dir().join(format!("molt-glob-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("sub/deep")).unwrap();
    for name in ["a.c", "b.c", "c.h", ".hidden.c", "sub/d.c", "sub/deep/e.c"] {
        std::fs::write(root.join(name), "").unwrap();
    }
    let dir = root.to_str().unwrap();
    check(&[
        (
            &format!("glob -directory {dir} -tails *.c") as &str,
            "a.c b.c",
        ),
        (
            &format!("glob -directory {dir} -tails *.{{c,h}}"),
            "a.c b.c c.h",
        ),
        (&format!("glob -directory {dir} -tails -types d *"), "sub"),
        (&format!("glob -directory {dir} -tails */*.c"), "sub/d.c"),
        (&format!("glob -directory {dir} -tails .*.c"), ".hidden.c"),
        (&format!("llength [glob {dir}/sub/*/*.c]"), "1"),
        (&format!("glob -nocomplain {dir}/*.zip"), ""),
    ]);
    fails(&[&format!("glob {dir}/*.zip")]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn logical_operators_accept_boolean_words() {
    check(&[
        ("expr {yes && 1}", "1"),
        ("expr {1 && no}", "0"),
        ("expr {false || on}", "1"),
        ("expr {!yes}", "0"),
        ("expr {!off}", "1"),
        ("set b true; expr {$b ? \"t\" : \"f\"}", "t"),
    ]);
    fails(&["expr {abc && 1}", "expr {!abc}"]);
}

#[test]
fn foreach_and_lmap_iterate_several_lists_in_parallel() {
    check(&[
        (
            "set r {}; foreach a {1 2 3} b {x y} {lappend r $a$b}; set r",
            "1x 2y 3",
        ),
        (
            "set r {}; foreach {a b} {1 2 3 4} c {x y} {lappend r $a$b$c}; set r",
            "12x 34y",
        ),
        ("lmap a {1 2} b {3 4} {expr {$a + $b}}", "4 6"),
    ]);
    fails(&["foreach a {1} b {}", "foreach {} {1} {}"]);
}
