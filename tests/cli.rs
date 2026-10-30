use std::process::{Command, Output};
struct Fixture {
    directory: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut index = String::new();
        for (name, fields) in [
            (
                "curl",
                "name curl version 8.0 revision 1 portdir net/curl categories {net www} description {Transfer data with URLs} homepage https://curl.se license curl variants {ssl universal} depends_lib {port:zlib}",
            ),
            (
                "zlib",
                "name zlib version 1.3 revision 0 portdir archivers/zlib categories archivers description {Compression library}",
            ),
            (
                "curlish",
                "name curlish version 1.0 revision 0 portdir net/curlish categories net description {A curl helper}",
            ),
        ] {
            index.push_str(&format!(
                "{name} {}\n{fields}\n",
                fields.encode_utf16().count() + 1
            ));
        }
        std::fs::write(directory.path().join("PortIndex"), index).unwrap();
        Self { directory }
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_port"))
            .args(args)
            .current_dir(self.directory.path())
            .env_remove("PORT_RS_INDEX")
            .env("HOME", self.directory.path().join("home"))
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
fn help_and_version_need_no_runtime_or_index() {
    for args in [vec!["--help"], vec!["--version"], vec!["help", "search"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_port"))
            .args(args)
            .current_dir(tempfile::tempdir().unwrap().path())
            .env("PATH", "")
            .output()
            .unwrap();
        assert!(output.status.success());
    }
}
#[test]
fn list_defaults_to_all_and_preserves_upstream_columns_even_when_quiet() {
    let fixture = Fixture::new();
    let actual = stdout(&fixture.run(&["list"]));
    assert_eq!(
        actual,
        format!(
            "{:<30} @{:<14} {}\n{:<30} @{:<14} {}\n{:<30} @{:<14} {}\n",
            "curl",
            "8.0",
            "net/curl",
            "curlish",
            "1.0",
            "net/curlish",
            "zlib",
            "1.3",
            "archivers/zlib"
        )
    );
    assert_eq!(stdout(&fixture.run(&["-q", "list", "all"])), actual);
}
#[test]
fn search_line_has_no_at_sign_and_uses_four_columns() {
    let fixture = Fixture::new();
    assert_eq!(
        stdout(&fixture.run(&["search", "--exact", "--line", "curl"])),
        "curl\t8.0\tnet www\tTransfer data with URLs\n"
    );
}
#[test]
fn search_modes_and_case() {
    let fixture = Fixture::new();
    assert_eq!(
        stdout(&fixture.run(&["-q", "search", "--exact", "CURL"])),
        "curl\n"
    );
    assert_eq!(
        stdout(&fixture.run(&["-q", "search", "--case-sensitive", "CURL"])),
        ""
    );
    assert_eq!(
        stdout(&fixture.run(&["-q", "search", "--regex", "^curl$"])),
        "curl\n"
    );
    assert_eq!(
        stdout(&fixture.run(&["-q", "search", "curl*"])),
        "curl\ncurlish\n"
    );
    assert_eq!(
        stdout(&fixture.run(&["-q", "search", "--name", "Compression"])),
        ""
    );
    assert_eq!(
        stdout(&fixture.run(&["-q", "search", "Compression"])),
        "zlib\n"
    );
}
#[test]
fn search_human_output_includes_revision_categories_and_summary() {
    let fixture = Fixture::new();
    assert_eq!(
        stdout(&fixture.run(&["search", "curl"])),
        "curl @8.0_1 (net, www)\n    Transfer data with URLs\n\ncurlish @1.0 (net)\n    A curl helper\n\nFound 2 ports.\n"
    );
    assert_eq!(
        stdout(&fixture.run(&["search", "missing"])),
        "No match for missing found\n"
    );
}
#[test]
fn search_multiple_patterns_keep_upstream_groups() {
    let fixture = Fixture::new();
    assert_eq!(
        stdout(&fixture.run(&["-q", "search", "--exact", "curl", "zlib"])),
        "curl\n--\nzlib\n"
    );
}
#[test]
fn info_index_field_query() {
    let fixture = Fixture::new();
    assert_eq!(
        stdout(&fixture.run(&["-q", "info", "--index", "--version", "curl"])),
        "8.0\n"
    );
    assert_eq!(
        stdout(&fixture.run(&["info", "--index", "--homepage", "curl"])),
        "homepage: https://curl.se\n"
    );
}
#[test]
fn selectors_are_deduplicated_and_unknown_names_fail() {
    let fixture = Fixture::new();
    assert_eq!(
        stdout(&fixture.run(&["-q", "echo", "category:net", "curl"])),
        "curl\ncurlish\n"
    );
    assert!(!fixture.run(&["list", "missing"]).status.success());
    assert!(!fixture.run(&["list", "installed"]).status.success());
}
#[test]
fn install_does_not_fake_success_or_invoke_external_port() {
    let fixture = Fixture::new();
    let output = fixture.run(&["install", "curl", "+ssl"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not implemented"));
    assert_eq!(
        std::fs::read_dir(fixture.directory.path()).unwrap().count(),
        1
    );
}
#[test]
fn malformed_index_fails_without_panicking() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.directory.path().join("PortIndex"),
        "curl 100\ntruncated\n",
    )
    .unwrap();
    let output = fixture.run(&["list"]);
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
}
#[test]
fn explicit_index_path_is_supported_without_conflicting_with_info_index() {
    let fixture = Fixture::new();
    let path = fixture.directory.path().join("PortIndex");
    assert_eq!(
        stdout(&fixture.run(&[
            "--index-file",
            path.to_str().unwrap(),
            "-q",
            "info",
            "--index",
            "--name",
            "curl"
        ])),
        "curl\n"
    );
}
#[test]
fn quiet_and_redirected_progress_emit_no_escape_codes() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.directory.path().join("empty.json"),
        r#"{"files":[]}"#,
    )
    .unwrap();
    let output = fixture.run(&["download", "--manifest", "empty.json"]);
    stdout(&output);
    assert!(!output.stderr.contains(&27));
    assert!(String::from_utf8_lossy(&output.stderr).contains("All downloads verified"));
    let output = fixture.run(&["-q", "download", "--manifest", "empty.json"]);
    assert!(output.stderr.is_empty());
    assert!(output.stdout.is_empty());
}

#[test]
fn explicit_missing_index_does_not_trigger_network_initialization() {
    let fixture = Fixture::new();
    let output = fixture.run(&["--index-file", "missing-index", "list"]);
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Initializing"));
    assert!(!fixture.directory.path().join("missing-index").exists());
}

#[test]
fn redirected_help_honors_no_color() {
    let fixture = Fixture::new();
    for args in [vec!["--help"], vec!["search", "--help"]] {
        let output = fixture.run(&args);
        let text = stdout(&output);
        assert!(text.contains("Usage:"));
        assert!(!text.contains('\x1b'));
    }
}

#[cfg(target_os = "macos")]
#[test]
fn default_index_uses_the_detected_platform_cache_offline() {
    let fixture = Fixture::new();
    let platform = port_rs::source::detect_platform().unwrap();
    let cache = platform.cache_path(&fixture.directory.path().join("home"));
    std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
    std::fs::rename(fixture.directory.path().join("PortIndex"), &cache).unwrap();
    let output = fixture.run(&["-q", "search", "--exact", "curl"]);
    assert_eq!(stdout(&output), "curl\n");
    assert!(output.stderr.is_empty());
}
