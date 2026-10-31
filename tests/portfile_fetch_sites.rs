use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::collections::BTreeMap;
#[test]
fn mirror_maps_keep_tags_environment_precedence_and_literal_command_arguments() {
    let mut s = Session::open(
        r#"
        name app; version 1; set global_mirror_site global
        set archive_sites {named:extra/:tbz2 https://literal/:txz}
        set archive_sites.mirror_subdir suffix
        proc lookup {sentinel class} {
            if {$sentinel ne {literal [not-a-command]}} {error command-prefix}
            if {$class eq "global"} {return {https://global/:nosubdir}}
            if {$class eq "named"} {return {https://named/:nosubdir https://second/:mirror}}
            return {}
        }
    "#,
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    s.invoke(
        "set",
        &[
            "env(ARCHIVE_SITE_LOCAL)",
            "https://local/:nosubdir https://tagged/:nosubdir:tbz2",
        ],
    )
    .unwrap();
    s.invoke(
        "portfetch::checksites",
        &[
            "archive_sites {global ARCHIVE_SITE_LOCAL}",
            "lookup {literal [not-a-command]}",
        ],
    )
    .unwrap();
    assert_eq!(
        s.invoke("set", &["portfetch::urlmap(tbz2)"]).unwrap(),
        "https://global/ https://local/ https://tagged/ https://named/extra/suffix https://second/app"
    );
    assert_eq!(
        s.invoke("set", &["portfetch::urlmap(txz)"]).unwrap(),
        "https://global/ https://local/ https://literal/"
    );
    assert_eq!(
        s.invoke("set", &["portfetch::urlmap(archive_sites)"])
            .unwrap(),
        "https://global/suffix https://local/"
    );
    assert_eq!(
        s.invoke(
            "portfetch::resolve_env_site",
            &["https://host/:mirror:tbz2"]
        )
        .unwrap(),
        "https://host/app:tbz2 1"
    );
    assert_eq!(
        s.invoke("portfetch::resolve_env_site", &["https://host/path"])
            .unwrap(),
        "https://host/path 0"
    );
}
#[test]
fn absent_lists_are_skipped_and_bad_lists_fail_without_hiding_errors() {
    let mut s = Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    s.invoke("portfetch::checksites", &["missing {}", "unused"])
        .unwrap();
    s.invoke("set", &["master_sites", "https://example/:tag"])
        .unwrap();
    s.invoke("set", &["portfetch::urlmap(tag)", "{"]).unwrap();
    assert!(
        s.invoke("portfetch::checksites", &["master_sites {}", "unused"])
            .is_err()
    );
}

#[test]
fn archive_checkfiles_joins_native_url_map_and_candidate_bookkeeping() {
    let mut s = Session::open(
        r#"
        name app; version 1; set global_mirror_site global
        set portdbpath /custom/db; set portarchivetype tbz2
        set os.platform darwin; set os.subplatform macosx; set os.major 24
        set os.arch arm; set configure.build_arch arm64
        archive_sites named::tbz2
        proc portarchivefetch::get_full_archive_sites_path {} {return {/tree with spaces/sites.tcl}}
        proc portarchivefetch::get_archive_site_urls {path class} {
            if {$path ne {/tree with spaces/sites.tcl} || $class ne "named"} {error wrong-resource}
            return {https://archive/:nosubdir}
        }
        proc collect {} {
            set local {}
            portarchivefetch::checkfiles local
            return $local
        }
    "#,
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    assert_eq!(
        s.invoke("collect", &[]).unwrap(),
        "tbz2 app-1_0.darwin_24.arm64.tbz2"
    );
    assert_eq!(
        s.invoke("set", &["portfetch::urlmap(tbz2)"]).unwrap(),
        "https://archive/"
    );
    assert_eq!(
        s.scalar("all_archive_files").unwrap(),
        "app-1_0.darwin_24.arm64.tbz2"
    );
}
