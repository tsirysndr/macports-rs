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

fn write_portfile(fixture: &Fixture, directory: &str, source: &str) {
    let path = fixture.0.path().join(directory);
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("Portfile"), source).unwrap();
}

#[test]
fn local_variants_evaluate_source_and_groups_instead_of_stale_index_metadata() {
    let fixture = Fixture::new(&[("example", "portdir net/example variants stale")]);
    let groups = fixture.0.path().join("_resources/port1.0/group");
    std::fs::create_dir_all(&groups).unwrap();
    std::fs::write(
        groups.join("helper-1.0.tcl"),
        "variant tls description {Enable TLS} {}; default_variants +tls",
    )
    .unwrap();
    write_portfile(
        &fixture,
        "net/example",
        r#"
        PortSystem 1.0
        name example
        version 2
        PortGroup helper 1.0
        if {${os.platform} eq "darwin"} {
            variant native description {Native build} {}
        }
        build {exec must-not-run}
    "#,
    );
    assert_eq!(
        stdout(fixture.run(&["variants", "example"])),
        "example has the variants:\n   native: Native build\n[+]tls: Enable TLS\n"
    );
    assert_eq!(
        stdout(fixture.run(&["-q", "variants", "example", "-tls"])),
        "   native: Native build\n  -tls: Enable TLS\n"
    );
    assert_eq!(
        stdout(fixture.run(&["-q", "variants", "--index", "example"])),
        "   stale\n"
    );
}

#[test]
fn local_variants_accept_explicit_tree_from_another_working_directory() {
    let fixture = Fixture::new(&[("example", "portdir net/example")]);
    write_portfile(
        &fixture,
        "net/example",
        "name example; version 1; variant local {}",
    );
    let elsewhere = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_port"))
        .args([
            "--index-file",
            fixture.0.path().join("PortIndex").to_str().unwrap(),
            "-q",
            "variants",
            "example",
        ])
        .current_dir(elsewhere.path())
        .env("PATH", "")
        .output()
        .unwrap();
    assert_eq!(stdout(output), "   local\n");
}

#[test]
fn local_variant_errors_never_print_partial_results_or_fall_back_to_index() {
    let fixture = Fixture::new(&[
        ("good", "portdir net/good"),
        ("bad", "portdir net/bad variants stale"),
    ]);
    write_portfile(
        &fixture,
        "net/good",
        "name good; version 1; variant local {}",
    );
    write_portfile(
        &fixture,
        "net/bad",
        "name bad; version 1; unsupported_portfile_command",
    );
    let output = fixture.run(&["variants", "good", "bad"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported_portfile_command"));
    std::fs::remove_file(fixture.0.path().join("net/bad/Portfile")).unwrap();
    let output = fixture.run(&["variants", "bad"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot find local Portfile"));
}

#[test]
fn local_variants_support_subports_and_user_options() {
    let fixture = Fixture::new(&[("child", "portdir net/parent")]);
    write_portfile(
        &fixture,
        "net/parent",
        r#"
        name parent
        version 1
        options feature
        feature no
        subport child {
            if {[option feature] eq "yes"} {variant selected {}}
        }
        subport other {variant wrong {}}
    "#,
    );
    assert_eq!(
        stdout(fixture.run(&["-q", "variants", "child", "feature=yes"])),
        "   selected\n"
    );
}

#[test]
fn local_portdir_cannot_traverse_outside_the_selected_tree() {
    for portdir in ["../outside", "/tmp/outside", "net/../../outside"] {
        let fixture = Fixture::new(&[("example", &format!("portdir {portdir}"))]);
        let output = fixture.run(&["variants", "example"]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("valid relative portdir"));
    }
    let fixture = Fixture::new(&[("example", "portdir outside")]);
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("Portfile"), "error must-not-evaluate").unwrap();
    std::os::unix::fs::symlink(outside.path(), fixture.0.path().join("outside")).unwrap();
    let output = fixture.run(&["variants", "example"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("outside the selected ports tree"));
}

#[test]
fn local_portfiles_require_bounded_utf8_regular_files() {
    let fixture = Fixture::new(&[("example", "portdir net/example")]);
    write_portfile(&fixture, "net/example", "");
    let path = fixture.0.path().join("net/example/Portfile");
    std::fs::write(&path, [0xff]).unwrap();
    let output = fixture.run(&["variants", "example"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot read"));
    std::fs::File::create(&path)
        .unwrap()
        .set_len(4 * 1024 * 1024 + 1)
        .unwrap();
    let output = fixture.run(&["variants", "example"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("4 MiB limit"));
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let output = fixture.run(&["variants", "example"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not a regular file"));
}
