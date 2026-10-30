//! Pinned, unmodified upstream sources. This exercises metadata evaluation with
//! explicit inputs; it does not execute PortSystem initialization or build hooks.
use port_rs::{
    portfile::{self, Evaluation, PortGroups},
    spec::PortSpec,
    tcl_list,
};
use std::collections::BTreeMap;

fn evaluate(modifiers: &[&str]) -> anyhow::Result<Evaluation> {
    let mut groups = PortGroups::default();
    groups.insert(
        "clang_dependency",
        "1.0",
        include_str!("fixtures/curl/clang_dependency-1.0.tcl").into(),
    )?;
    groups.insert(
        "muniversal",
        "1.0",
        include_str!("fixtures/curl/muniversal-1.0.tcl").into(),
    )?;
    let variables = [
        ("prefix", "/opt/local"),
        ("os.platform", "darwin"),
        ("os.subplatform", "macosx"),
        ("os.major", "24"),
        ("os.arch", "arm"),
        ("macosx_deployment_target", "15.0"),
        ("build.jobs", "4"),
        ("configure.universal_archs", "arm64 x86_64"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect::<BTreeMap<_, _>>();
    let mut spec = PortSpec::named("curl");
    for modifier in modifiers {
        spec.parse_modifier(modifier)?;
    }
    portfile::evaluate_with_groups(
        include_str!("fixtures/curl/Portfile"),
        &variables,
        &spec,
        groups,
    )
}

#[test]
fn resolves_real_curl_default_variants_dependencies_and_livecheck_metadata() {
    let value = evaluate(&[]).unwrap();
    assert_eq!(value.enabled, "+brotli+http2+idn+psl+ssl+zstd");
    assert_eq!(value.port.get("version"), "8.23.0");
    assert_eq!(
        tcl_list::parse(value.port.get("depends_lib")).unwrap(),
        [
            "port:zlib",
            "port:brotli",
            "port:nghttp2",
            "port:libidn2",
            "port:libpsl",
            "path:lib/libssl.dylib:openssl",
            "path:share/curl/curl-ca-bundle.crt:curl-ca-bundle",
            "port:zstd",
        ]
    );
    assert_eq!(
        value
            .groups
            .iter()
            .map(|g| g.name.as_str())
            .collect::<Vec<_>>(),
        ["clang_dependency", "muniversal"]
    );
    assert_eq!(value.port.get("livecheck.url"), "https://curl.se/download/");
    assert_eq!(
        tcl_list::parse(value.port.get("livecheck.regex")).unwrap(),
        [r"curl-([0-9.]+)\.tar\.xz"]
    );
    let variants = tcl_list::dictionary(value.port.get("vinfo")).unwrap();
    assert!(variants.contains_key("ares"));
    assert!(
        !tcl_list::dictionary(&variants["ares"])
            .unwrap()
            .contains_key("is_default")
    );
    assert_eq!(
        tcl_list::dictionary(&variants["ssl"]).unwrap()["is_default"],
        "+"
    );
    assert!(
        value.hooks.iter().any(|hook| hook.phase == "pre-activate"
            && hook.body.contains("registry_deactivate_composite"))
    );
    assert!(
        value
            .hooks
            .iter()
            .any(|hook| hook.phase == "post-destroot" && hook.body.contains("xinstall"))
    );
}

#[test]
fn real_curl_explicit_variants_change_tls_and_resolver_dependencies() {
    let value = evaluate(&["+ares", "+gnutls", "-brotli"]).unwrap();
    assert_eq!(value.enabled, "+ares+gnutls+http2+idn+psl+zstd");
    let deps = tcl_list::parse(value.port.get("depends_lib")).unwrap();
    assert!(deps.iter().any(|d| d == "port:c-ares"));
    assert!(deps.iter().any(|d| d.ends_with(":gnutls")));
    assert!(
        !deps
            .iter()
            .any(|d| d.ends_with(":openssl") || d == "port:brotli")
    );
    let args = tcl_list::parse(value.port.get("configure.args")).unwrap();
    assert!(args.iter().any(|a| a == "--enable-ares"));
    assert!(!args.iter().any(|a| a == "--disable-ares"));
}

#[test]
fn real_curl_requirements_and_conflicts_are_enforced() {
    let value = evaluate(&["+http3"]).unwrap();
    assert!(value.enabled.contains("+gnutls"));
    assert!(!value.enabled.contains("+ssl"));
    let deps = tcl_list::parse(value.port.get("depends_lib")).unwrap();
    assert!(deps.iter().any(|d| d == "port:nghttp3"));
    for modifiers in [["+ssl", "+gnutls"], ["+psl", "-idn"]] {
        assert!(evaluate(&modifiers).is_err(), "{modifiers:?}");
    }
}

#[test]
fn cli_queries_real_curl_with_detected_and_explicit_build_inputs() {
    let tree = tempfile::tempdir().unwrap();
    let portdir = tree.path().join("net/curl");
    let groupdir = tree.path().join("_resources/port1.0/group");
    std::fs::create_dir_all(&portdir).unwrap();
    std::fs::create_dir_all(&groupdir).unwrap();
    std::fs::write(
        portdir.join("Portfile"),
        include_str!("fixtures/curl/Portfile"),
    )
    .unwrap();
    std::fs::write(
        groupdir.join("clang_dependency-1.0.tcl"),
        include_str!("fixtures/curl/clang_dependency-1.0.tcl"),
    )
    .unwrap();
    std::fs::write(
        groupdir.join("muniversal-1.0.tcl"),
        include_str!("fixtures/curl/muniversal-1.0.tcl"),
    )
    .unwrap();
    let fields = "name curl version 8.23.0 portdir net/curl variants stale";
    std::fs::write(
        tree.path().join("PortIndex"),
        format!("curl {}\n{fields}\n", fields.len() + 1),
    )
    .unwrap();
    for action in ["variants", "deps"] {
        for explicit in [false, true] {
            let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_port"));
            command.args([action, "curl"]);
            if explicit {
                command.args([
                    "macosx_deployment_target=15.0",
                    "build.jobs=4",
                    "configure.universal_archs=arm64 x86_64",
                ]);
            }
            let output = command
                .current_dir(tree.path())
                .env_remove("PORT_RS_INDEX")
                .env("PATH", "")
                .env("NO_COLOR", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let stdout = String::from_utf8(output.stdout).unwrap();
            if action == "variants" {
                assert!(stdout.contains("[+]ssl:"), "{stdout}");
                assert!(stdout.contains("   ares:"), "{stdout}");
                assert!(!stdout.contains("stale"));
            } else {
                assert!(
                    stdout.contains("curl @8.23.0_0+brotli+http2+idn+psl+ssl+zstd"),
                    "{stdout}"
                );
                assert!(stdout.contains("openssl"));
            }
        }
    }
}
