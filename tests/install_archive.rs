#[path = "support/archives.rs"]
mod archives;
use archives::*;
use port_rs::install::inspect::{self, Kind, Limits};

#[test]
fn inspects_authenticated_metadata_files_and_links_without_extracting() {
    let c = contents(&[
        "opt/local/bin/app",
        "opt/local/bin/alias",
        "opt/local/bin/hard",
    ]);
    let result = inspect_records(&[
        ("./", b'5', b"", ""),
        ("./+CONTENTS", b'0', c.as_bytes(), ""),
        ("opt/local/bin/app", b'0', b"payload", ""),
        ("opt/local/bin/alias", b'2', b"", "/opt/local/bin/app"),
        ("opt/local/bin/hard", b'1', b"", "opt/local/bin/app"),
    ])
    .unwrap();
    assert_eq!(result.contents.name, "app");
    assert_eq!(result.contents.version, "1");
    assert_eq!(result.contents.archs, ["arm64"]);
    assert_eq!(result.contents.dependencies, ["base-2_0"]);
    assert_eq!(result.members["opt/local/bin/app"].size, 7);
    assert_eq!(
        result.members["opt/local/bin/alias"].kind,
        Kind::Symlink("/opt/local/bin/app".into())
    );
}
#[test]
fn rejects_traversal_absolute_paths_and_duplicate_normalized_paths() {
    let c = contents(&["opt/local/app"]);
    for path in [
        "../escape",
        "/absolute",
        "opt/../escape",
        "opt\\escape",
        "opt/line\nbreak",
    ] {
        assert!(
            inspect_records(&[
                ("+CONTENTS", b'0', c.as_bytes(), ""),
                (path, b'0', b"x", "")
            ])
            .is_err(),
            "{path}"
        );
    }
    for duplicate in ["./opt/local/app", "opt/local/APP"] {
        assert!(
            inspect_records(&[
                ("+CONTENTS", b'0', c.as_bytes(), ""),
                ("opt/local/app", b'0', b"x", ""),
                (duplicate, b'0', b"y", "")
            ])
            .is_err()
        );
    }
}
#[test]
fn rejects_descendants_of_links_even_when_parent_comes_last() {
    let c = contents(&["opt/link/child", "opt/LINK"]);
    assert!(
        inspect_records(&[
            ("+CONTENTS", b'0', c.as_bytes(), ""),
            ("opt/link/child", b'0', b"x", ""),
            ("opt/LINK", b'2', b"", "/tmp")
        ])
        .unwrap_err()
        .to_string()
        .contains("non-directory")
    );
}
#[test]
fn rejects_special_files_extensions_and_hardlinks_to_nonfiles() {
    let c = contents(&["opt/local/app"]);
    for kind in *b"346xgLS" {
        assert!(
            inspect_records(&[
                ("+CONTENTS", b'0', c.as_bytes(), ""),
                ("opt/local/app", kind, b"", "")
            ])
            .is_err()
        );
    }
    for target in ["missing", "+CONTENTS", "opt/local/app"] {
        assert!(
            inspect_records(&[
                ("+CONTENTS", b'0', c.as_bytes(), ""),
                ("opt/local/app", b'1', b"", target)
            ])
            .is_err()
        );
    }
}
#[test]
fn validates_inventory_and_unique_identity_metadata() {
    let c = contents(&["opt/local/app"]);
    assert!(inspect_records(&[("opt/local/app", b'0', b"x", "")]).is_err());
    assert!(inspect_records(&[("+CONTENTS", b'0', c.as_bytes(), "")]).is_err());
    assert!(
        inspect_records(&[
            ("+CONTENTS", b'0', contents(&[]).as_bytes(), ""),
            ("opt/local/app", b'0', b"x", "")
        ])
        .is_err()
    );
    for invalid in [
        format!("{c}@portname other\n"),
        c.replace("@name app-1_0+ssl", "@name other-1_0+ssl"),
        c.replace("@portrevision 0", "@portrevision -1"),
        format!("{c}@exec touch /tmp/evil\n"),
        format!("{c}@ignore\n"),
    ] {
        assert!(
            inspect_records(&[
                ("+CONTENTS", b'0', invalid.as_bytes(), ""),
                ("opt/local/app", b'0', b"x", "")
            ])
            .is_err()
        );
    }
}
#[test]
fn controls_are_inert_regular_files_and_not_payload() {
    let c = format!("{}@ignore\n+PORTFILE\n", contents(&[]));
    assert!(
        inspect_records(&[
            ("+CONTENTS", b'0', c.as_bytes(), ""),
            ("+PORTFILE", b'0', b"exec dangerous-code", "")
        ])
        .is_ok()
    );
    assert!(
        inspect_records(&[
            ("+CONTENTS", b'0', c.as_bytes(), ""),
            ("+PORTFILE", b'2', b"", "/etc/passwd")
        ])
        .is_err()
    );
}
#[test]
fn bounds_expansion_entry_count_and_contents_size() {
    let c = contents(&["opt/local/app"]);
    let bytes = compress(&tar(&[
        ("+CONTENTS", b'0', c.as_bytes(), ""),
        ("opt/local/app", b'0', &vec![0; 100_000], ""),
    ]));
    for limits in [
        Limits {
            expanded_bytes: 2000,
            ..Limits::default()
        },
        Limits {
            entries: 1,
            ..Limits::default()
        },
        Limits {
            contents_bytes: 10,
            ..Limits::default()
        },
    ] {
        assert!(inspect::tbz2(&mut stage(&bytes), limits).is_err());
    }
}
#[test]
fn rejects_truncated_bzip_corruption_and_trailing_streams() {
    let c = contents(&[]);
    let bytes = compress(&tar(&[("+CONTENTS", b'0', c.as_bytes(), "")]));
    let mut corrupt = bytes.clone();
    let n = corrupt.len();
    corrupt[n - 5] ^= 1;
    let mut trailing = bytes.clone();
    trailing.extend(&bytes);
    for invalid in [bytes[..bytes.len() - 5].to_vec(), corrupt, trailing] {
        assert!(inspect::tbz2(&mut stage(&invalid), Limits::default()).is_err());
    }
    let mut raw = tar(&[("+CONTENTS", b'0', c.as_bytes(), "")]);
    raw.extend(b"hidden");
    assert!(inspect::tbz2(&mut stage(&compress(&raw)), Limits::default()).is_err());
}
#[test]
fn binds_identity_to_requested_installation_item() {
    let c = contents(&[]);
    let result = inspect_records(&[("+CONTENTS", b'0', c.as_bytes(), "")]).unwrap();
    let body = "name app version 1 revision 0 variants ssl vinfo {ssl {is_default +}}";
    let index = port_rs::index::Index::parse(&format!("app {}\n{body}\n", body.len() + 1)).unwrap();
    let mut plan =
        port_rs::install::plan(&index, &[port_rs::spec::PortSpec::named("app")], &[]).unwrap();
    result.contents.matches(&plan[0]).unwrap();
    plan[0].variants.clear();
    assert!(result.contents.matches(&plan[0]).is_err());
}

fn pax(key: &str, value: &str) -> Vec<u8> {
    let body = format!(" {key}={value}\n");
    let mut length = body.len() + 1;
    loop {
        let next = body.len() + length.to_string().len();
        if next == length {
            break;
        }
        length = next;
    }
    format!("{length}{body}").into_bytes()
}
#[test]
fn bounded_pax_overrides_are_applied_before_path_validation() {
    let c = contents(&["opt/local/app"]);
    let p = pax("path", "opt/local/app");
    let result = inspect_records(&[
        ("+CONTENTS", b'0', c.as_bytes(), ""),
        ("PaxHeader", b'x', &p, ""),
        ("placeholder", b'0', b"x", ""),
    ])
    .unwrap();
    assert!(result.members.contains_key("opt/local/app"));
    for invalid in ["../escape", "/absolute"] {
        let p = pax("path", invalid);
        assert!(
            inspect_records(&[
                ("+CONTENTS", b'0', c.as_bytes(), ""),
                ("PaxHeader", b'x', &p, ""),
                ("placeholder", b'0', b"x", "")
            ])
            .is_err()
        );
    }
}
#[test]
fn pax_retains_macos_flags_and_effective_link_targets() {
    let c = contents(&["opt/local/link"]);
    let mut p = pax("linkpath", "/opt/local/target");
    p.extend(pax("SCHILY.fflags", "compressed"));
    let result = inspect_records(&[
        ("+CONTENTS", b'0', c.as_bytes(), ""),
        ("PaxHeader", b'x', &p, ""),
        ("opt/local/link", b'2', b"", "placeholder"),
    ])
    .unwrap();
    assert_eq!(
        result.members["opt/local/link"].kind,
        Kind::Symlink("/opt/local/target".into())
    );
    assert_eq!(
        result.members["opt/local/link"].attributes["SCHILY.fflags"],
        "compressed"
    );
}
#[test]
fn pax_rejects_unsupported_sizes_malformed_records_and_oversized_metadata() {
    let c = contents(&["opt/local/app"]);
    let mut duplicates = pax("path", "opt/local/app");
    duplicates.extend(pax("path", "other"));
    for p in [
        pax("size", "1"),
        pax("GNU.sparse.size", "100"),
        pax("uname", &"x".repeat(65536)),
        duplicates,
        b"\n10 path=x\n".to_vec(),
        b"99 path=x\n".to_vec(),
    ] {
        assert!(
            inspect_records(&[
                ("+CONTENTS", b'0', c.as_bytes(), ""),
                ("PaxHeader", b'x', &p, ""),
                ("opt/local/app", b'0', b"x", "")
            ])
            .is_err()
        );
    }
}

#[test]
fn unresolved_plan_variants_cannot_approve_an_archive() {
    let c = contents(&[]);
    let result = inspect_records(&[("+CONTENTS", b'0', c.as_bytes(), "")]).unwrap();
    let body = "name app version 1 variants ssl";
    let index = port_rs::index::Index::parse(&format!("app {}\n{body}\n", body.len() + 1)).unwrap();
    let plan =
        port_rs::install::plan(&index, &[port_rs::spec::PortSpec::named("app")], &[]).unwrap();
    assert!(
        result
            .contents
            .matches(&plan[0])
            .unwrap_err()
            .to_string()
            .contains("unresolved")
    );
}
