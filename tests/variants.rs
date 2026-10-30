use std::process::{Command, Output};

struct Fixture(tempfile::TempDir);
impl Fixture {
    fn new(records: &[(&str, &str)]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut data = String::new();
        for (name, fields) in records {
            let body = format!("name {name} version 1 revision 0 {fields}");
            data.push_str(&format!(
                "{name} {}\n{body}\n",
                body.encode_utf16().count() + 1
            ));
        }
        std::fs::write(directory.path().join("PortIndex"), data).unwrap();
        Self(directory)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_port"))
            .args(args)
            .current_dir(self.0.path())
            .env_remove("PORT_RS_INDEX")
            .env("HOME", self.0.path().join("home"))
            .env("PATH", "")
            .env("NO_COLOR", "1")
            .output()
            .unwrap()
    }
}
fn stdout(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

const METADATA: &str = "variants {zeta alpha bare} vinfo {alpha {description {Enable alpha support} is_default + conflicts { zeta } requires { bare }} zeta {description {Use zeta instead} is_default -}}";

#[test]
fn indexed_variants_show_sorted_descriptions_defaults_and_relationships() {
    let fixture = Fixture::new(&[("example", METADATA)]);
    assert_eq!(
        stdout(fixture.run(&["variants", "--index", "EXAMPLE"])),
        "example has the variants:\n[+]alpha: Enable alpha support\n     * conflicts with zeta\n     * requires bare\n   bare\n[-]zeta: Use zeta instead\n"
    );
}

#[test]
fn requested_markers_override_defaults_without_evaluating_metadata() {
    let fixture = Fixture::new(&[("example", METADATA)]);
    assert_eq!(
        stdout(fixture.run(&[
            "-q",
            "variants",
            "--index",
            "example@99",
            "-alpha",
            "+zeta",
            "+bare",
            "build.jobs=8"
        ])),
        "  -alpha: Enable alpha support\n     * conflicts with zeta\n     * requires bare\n   bare\n  +zeta: Use zeta instead\n"
    );
}

#[test]
fn absent_and_empty_variants_have_distinct_headers_and_quiet_behavior() {
    let fixture = Fixture::new(&[("absent", ""), ("empty", "variants {}")]);
    assert_eq!(
        stdout(fixture.run(&["variants", "--index", "absent", "empty"])),
        "absent has no variants\nempty has the variants:\n"
    );
    assert_eq!(
        stdout(fixture.run(&["-q", "variants", "--index", "all"])),
        ""
    );
}

#[test]
fn variant_names_use_ascii_sorting_and_keep_duplicates() {
    let fixture = Fixture::new(&[("example", "variants {x2 x10 x2 A a}")]);
    assert_eq!(
        stdout(fixture.run(&["-q", "variants", "--index", "example"])),
        "   A\n   a\n   x10\n   x2\n   x2\n"
    );
}

#[test]
fn malformed_later_roots_fail_without_partial_output() {
    for fields in [
        "variants {\"unterminated}",
        "variants alpha vinfo {alpha}",
        "variants alpha vinfo {alpha {description}}",
    ] {
        let fixture = Fixture::new(&[("good", METADATA), ("bad", fields)]);
        let output = fixture.run(&["variants", "--index", "good", "bad"]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("bad: variant metadata"));
    }
}

#[test]
fn non_index_variant_queries_fail_before_catalog_initialization() {
    let fixture = Fixture::new(&[]);
    std::fs::remove_file(fixture.0.path().join("PortIndex")).unwrap();
    let output = fixture.run(&["variants", "example"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("use variants --index"));
    assert!(!fixture.0.path().join("home").exists());
}

#[test]
fn variant_queries_support_expressions_abbreviations_and_chains() {
    let fixture = Fixture::new(&[("example", "variants ssl"), ("other", "variants debug")]);
    assert_eq!(
        stdout(fixture.run(&[
            "-q", "var", "--index", "all", "and", "not", "other", ";", "var", "--index", "other"
        ])),
        "   ssl\n   debug\n"
    );
}
