use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::collections::BTreeMap;
fn session() -> Session {
    Session::open("name app; version 1; variant ssl {}; default_variants +ssl; set portarchivetype tbz2; set os.platform darwin; set os.subplatform macosx; set os.major 24; set os.arch arm; set configure.build_arch arm64; set portdbpath /custom/db", &BTreeMap::new(), &PortSpec::named("app"), PortGroups::default()).unwrap()
}
#[test]
fn image_names_use_live_platform_architectures_variants_and_archive_type() {
    let mut s = session();
    assert_eq!(
        s.invoke("get_portimage_name", &[]).unwrap(),
        "app-1_0+ssl.darwin_24.arm64.tbz2"
    );
    s.invoke("platforms", &["macosx any"]).unwrap();
    assert_eq!(
        s.invoke("_get_archive_platform", &[]).unwrap(),
        "darwin_any"
    );
    s.invoke("set", &["portarchivetype", "txz"]).unwrap();
    s.invoke("supported_archs", &["noarch"]).unwrap();
    assert_eq!(
        s.invoke("get_portimage_name", &[]).unwrap(),
        "app-1_0+ssl.darwin_any.noarch.txz"
    );
    s.invoke("platforms", &["any"]).unwrap();
    assert_eq!(s.invoke("_get_archive_platform", &[]).unwrap(), "any_any");
    let variants = format!("+{}", "v".repeat(250));
    s.invoke("set", &["portvariants", &variants]).unwrap();
    use ripemd::{Digest, Ripemd160};
    assert_eq!(
        s.invoke("get_portimage_name", &[]).unwrap(),
        format!(
            "app-1_0+{}.any_any.noarch.txz",
            Ripemd160::digest(variants.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        )
    );
    s.invoke("version", &[&"1".repeat(250)]).unwrap();
    assert!(
        s.invoke("get_portimage_name", &[])
            .unwrap_err()
            .to_string()
            .contains("filename too long")
    );
}
#[test]
fn archive_file_candidates_preserve_type_order_and_caller_local_lists() {
    let mut s = session();
    s.invoke(
        "archive_sites",
        &[
            "first::txz",
            "second::tbz2",
            "duplicate::txz",
            "https://mirror/:tgz",
        ],
    )
    .unwrap();
    s.invoke(
        "proc",
        &[
            "prepare",
            "",
            "set urls {existing value}; portarchivefetch::checkarchivefiles urls; return $urls",
        ],
    )
    .unwrap();
    let expected = "existing value txz app-1_0+ssl.darwin_24.arm64.txz tbz2 app-1_0+ssl.darwin_24.arm64.tbz2 tgz app-1_0+ssl.darwin_24.arm64.tgz";
    assert_eq!(s.invoke("prepare", &[]).unwrap(), expected);
    assert_eq!(s.invoke("info", &["exists", "urls"]).unwrap(), "0");
    assert_eq!(
        s.scalar("archivefetch.fulldestpath").unwrap(),
        "/custom/db/incoming/verified"
    );
    assert_eq!(
        s.scalar("all_archive_files").unwrap(),
        "app-1_0+ssl.darwin_24.arm64.txz app-1_0+ssl.darwin_24.arm64.tbz2 app-1_0+ssl.darwin_24.arm64.tgz"
    );
    s.invoke("prepare", &[]).unwrap();
    assert_eq!(
        port_rs::tcl_list::parse(&s.scalar("all_archive_files").unwrap())
            .unwrap()
            .len(),
        6
    );
}
#[test]
fn empty_sites_leave_lists_untouched_and_errors_propagate() {
    let mut s = session();
    s.invoke("archive_sites", &[]).unwrap();
    s.invoke("portarchivefetch::checkarchivefiles", &["urls"])
        .unwrap();
    assert_eq!(s.invoke("info", &["exists", "urls"]).unwrap(), "0");
    assert_eq!(
        s.invoke("info", &["exists", "all_archive_files"]).unwrap(),
        "0"
    );
    s.invoke("archive_sites", &["site::tbz2"]).unwrap();
    s.invoke("set", &["urls", "{malformed"]).unwrap();
    assert!(
        s.invoke("portarchivefetch::checkarchivefiles", &["urls"])
            .is_err()
    );
    assert_eq!(s.scalar("urls").unwrap(), "{malformed");
    assert!(s.invoke("get_portimage_name", &["extra"]).is_err());
}

#[test]
fn evaluated_variant_globals_match_archive_identity_and_filter_requested_inputs() {
    let mut spec = PortSpec::named("app");
    for modifier in ["+zlib", "-debug", "+missing", "-unknown"] {
        spec.parse_modifier(modifier).unwrap();
    }
    let mut s = Session::open("name app; version 1; variant zlib requires ssl {}; variant ssl {}; variant debug {}; variant defaulted {}; default_variants +defaulted; set portvariants stale; set requested_variants stale", &BTreeMap::new(), &spec, PortGroups::default()).unwrap();
    assert_eq!(s.scalar("portvariants").unwrap(), "+defaulted+ssl+zlib");
    assert_eq!(s.scalar("requested_variants").unwrap(), "+zlib-debug");
    assert_eq!(
        s.active_variants().unwrap(),
        s.scalar("portvariants").unwrap()
    );
    assert_eq!(
        s.metadata().port.get("canonical_active_variants"),
        "+defaulted+ssl+zlib"
    );
    let mut empty = Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    assert_eq!(empty.scalar("portvariants").unwrap(), "");
    assert_eq!(empty.scalar("requested_variants").unwrap(), "");
}
