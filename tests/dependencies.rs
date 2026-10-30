use port_rs::{
    dependencies::{Filter, Graph},
    index::Index,
};
use std::process::{Command, Output};

fn index_data(records: &[(&str, &str)]) -> String {
    let mut data = String::new();
    for (name, fields) in records {
        let body = format!("name {name} version 1.0 revision 0 {fields}");
        data.push_str(&format!(
            "{name} {}\n{body}\n",
            body.encode_utf16().count() + 1
        ));
    }
    data
}
fn walk(records: &[(&str, &str)], root: &str, full: bool) -> Vec<(String, usize, bool)> {
    let index = Index::parse(&index_data(records)).unwrap();
    let graph = Graph::build(&index, root, Filter::default()).unwrap();
    let mut rows = Vec::new();
    graph
        .walk(full, |d, depth, cycle| {
            rows.push((d.name.clone(), depth, cycle));
            Ok(())
        })
        .unwrap();
    rows
}
#[test]
fn shared_dependencies_are_deduplicated_unless_full_is_requested() {
    let records = [
        ("root", "depends_lib {port:left port:right}"),
        ("left", "depends_lib port:shared"),
        ("right", "depends_run port:shared"),
        ("shared", ""),
    ];
    assert_eq!(
        walk(&records, "root", false),
        [
            ("left".into(), 1, false),
            ("shared".into(), 2, false),
            ("right".into(), 1, false)
        ]
    );
    assert_eq!(
        walk(&records, "root", true),
        [
            ("left".into(), 1, false),
            ("shared".into(), 2, false),
            ("right".into(), 1, false),
            ("shared".into(), 2, false)
        ]
    );
}
#[test]
fn cycles_terminate_and_full_output_marks_them() {
    let records = [("a", "depends_lib port:b"), ("b", "depends_lib port:a")];
    assert_eq!(
        walk(&records, "a", false),
        [("b".into(), 1, false), ("a".into(), 2, false)]
    );
    assert_eq!(
        walk(&records, "a", true),
        [
            ("b".into(), 1, false),
            ("a".into(), 2, false),
            ("b".into(), 3, true)
        ]
    );
    assert_eq!(
        walk(&[("a", "depends_lib port:a")], "a", true),
        [("a".into(), 1, false), ("a".into(), 2, true)]
    );
}
#[test]
fn recursive_filters_apply_to_every_node() {
    let index = Index::parse(&index_data(&[
        ("root", "depends_run port:child depends_build port:missing"),
        (
            "child",
            "depends_build port:also-missing depends_test port:test-missing",
        ),
    ]))
    .unwrap();
    assert!(Graph::build(&index, "root", Filter::default()).is_err());
    let graph = Graph::build(
        &index,
        "root",
        Filter {
            no_build: true,
            no_test: true,
        },
    )
    .unwrap();
    let mut names = Vec::new();
    graph
        .walk(false, |d, _, _| {
            names.push(d.name.clone());
            Ok(())
        })
        .unwrap();
    assert_eq!(names, ["child"]);
}
#[test]
fn missing_dependency_errors_identify_parent_and_specification() {
    let index = Index::parse(&index_data(&[(
        "root",
        "depends_lib path:lib/missing:missing",
    )]))
    .unwrap();
    let error = Graph::build(&index, "root", Filter::default())
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("root depends on path:lib/missing:missing"));
}
#[test]
fn deep_graphs_do_not_use_the_call_stack() {
    let records: Vec<(String, String)> = (0..5000)
        .map(|i| {
            (
                format!("p{i}"),
                if i < 4999 {
                    format!("depends_lib port:p{}", i + 1)
                } else {
                    String::new()
                },
            )
        })
        .collect();
    let records: Vec<_> = records
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_str()))
        .collect();
    let rows = walk(&records, "p0", false);
    assert_eq!(rows.len(), 4999);
    assert_eq!(rows.last().unwrap(), &("p4999".into(), 4999, false));
}
#[test]
fn names_are_resolved_case_insensitively() {
    assert_eq!(
        walk(
            &[("root", "depends_lib port:LEAF"), ("leaf", "")],
            "ROOT",
            false
        ),
        [("LEAF".into(), 1, false)]
    );
}

struct Fixture(tempfile::TempDir);
impl Fixture {
    fn new(records: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("PortIndex"), index_data(records)).unwrap();
        Self(dir)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_port"))
            .args(args)
            .current_dir(self.0.path())
            .env_remove("PORT_RS_INDEX")
            .env("HOME", self.0.path())
            .env("PATH", "")
            .env("NO_COLOR", "1")
            .output()
            .unwrap()
    }
}
fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}
#[test]
fn direct_dependencies_use_phase_labels_and_verbosity() {
    let fixture = Fixture::new(&[(
        "root",
        "depends_fetch port:fetcher depends_extract port:extractor depends_patch port:patcher depends_build bin:make:gmake depends_lib {port:zlib lib:libssl:openssl} depends_run path:bin/tool:runtime depends_test port:tester",
    )]);
    assert_eq!(
        stdout(&fixture.run(&["deps", "--index", "root"])),
        "Full Name: root @1.0_0\nFetch Dependencies:   fetcher\nExtract Dependencies: extractor\nPatch Dependencies:   patcher\nBuild Dependencies:   gmake\nLibrary Dependencies: zlib, openssl\nRuntime Dependencies: runtime\nTest Dependencies:    tester\n"
    );
    assert_eq!(
        stdout(&fixture.run(&[
            "-q",
            "-v",
            "deps",
            "--index",
            "--no-build",
            "--no-test",
            "root"
        ])),
        "Library Dependencies: port:zlib, lib:libssl:openssl\nRuntime Dependencies: path:bin/tool:runtime\n"
    );
}
#[test]
fn recursive_cli_supports_full_quiet_and_verbose_modes() {
    let fixture = Fixture::new(&[
        ("root", "depends_lib {port:left port:right}"),
        ("left", "depends_lib port:shared"),
        ("right", "depends_lib port:shared"),
        ("shared", ""),
    ]);
    assert_eq!(
        stdout(&fixture.run(&["rdeps", "--index", "root"])),
        "The following ports are dependencies of root @1.0_0:\n  left\n    shared\n  right\n"
    );
    assert_eq!(
        stdout(&fixture.run(&["-q", "-v", "rdeps", "--index", "--full", "root"])),
        "  port:left\n    port:shared\n  port:right\n    port:shared\n"
    );
}
#[test]
fn recursive_cli_marks_cycles() {
    let fixture = Fixture::new(&[("a", "depends_lib port:b"), ("b", "depends_run port:a")]);
    assert_eq!(
        stdout(&fixture.run(&["-q", "rdeps", "--index", "--full", "a"])),
        "  b\n    a\n      b (cyclic dependency)\n"
    );
}
#[test]
fn recursive_cli_filters_all_build_and_test_edges() {
    let fixture = Fixture::new(&[
        ("root", "depends_fetch port:missing depends_run port:child"),
        (
            "child",
            "depends_build port:missing depends_test port:missing",
        ),
    ]);
    assert_eq!(
        stdout(&fixture.run(&["-q", "rdeps", "--index", "--no-build", "--no-test", "root"])),
        "  child\n"
    );
}
#[test]
fn leaves_and_multiple_roots_keep_upstream_headers_and_separators() {
    let fixture = Fixture::new(&[("a", ""), ("b", "")]);
    for command in ["deps", "rdeps"] {
        assert_eq!(
            stdout(&fixture.run(&[command, "--index", "a", "b"])),
            "a @1.0_0 has no dependencies.\n--\nb @1.0_0 has no dependencies.\n"
        );
        assert_eq!(stdout(&fixture.run(&["-q", command, "--index", "a"])), "");
    }
}
#[test]
fn missing_and_malformed_dependencies_fail_without_partial_tree_output() {
    for dependencies in ["depends_lib port:missing", "depends_lib invalid"] {
        let fixture = Fixture::new(&[("root", dependencies)]);
        let output = fixture.run(&["rdeps", "--index", "root"]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
    }
}
#[test]
fn evaluation_requests_fail_explicitly() {
    let fixture = Fixture::new(&[("root", "")]);
    for args in [
        vec!["deps", "root"],
        vec!["rdeps", "root"],
        vec!["deps", "root", "+ssl"],
        vec!["rdeps", "root@2"],
    ] {
        let output = fixture.run(&args);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Portfile evaluation"));
    }
}

#[test]
fn explicit_index_mode_uses_available_metadata_for_qualified_specs() {
    let fixture = Fixture::new(&[("root", "depends_lib port:leaf"), ("leaf", "")]);
    assert_eq!(
        stdout(&fixture.run(&["deps", "--index", "root@99", "+ssl"])),
        "Full Name: root @1.0_0\nLibrary Dependencies: leaf\n"
    );
    assert_eq!(
        stdout(&fixture.run(&["-q", "rdeps", "--index", "root", "-ssl"])),
        "  leaf\n"
    );
}
