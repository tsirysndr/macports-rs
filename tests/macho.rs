use port_rs::macho::architectures;
use std::io::Cursor;
fn thin(cpu: u32, little: bool, wide: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    for word in [
        if wide { 0xfeedfacf } else { 0xfeedface },
        cpu,
        0,
        1,
        0,
        0,
        0,
    ] {
        bytes.extend(if little {
            u32::to_le_bytes(word)
        } else {
            u32::to_be_bytes(word)
        });
    }
    if wide {
        bytes.extend([0; 4]);
    }
    bytes
}
fn inspect(bytes: Vec<u8>) -> Vec<String> {
    architectures(&mut Cursor::new(bytes))
        .unwrap()
        .into_iter()
        .collect()
}
fn fat(slices: &[Vec<u8>], wide: bool) -> Vec<u8> {
    let entry_size = if wide { 32 } else { 20 };
    let mut bytes = Vec::new();
    bytes.extend(u32::to_be_bytes(if wide { 0xcafebabf } else { 0xcafebabe }));
    bytes.extend((slices.len() as u32).to_be_bytes());
    let mut offset = 8 + slices.len() * entry_size;
    for slice in slices {
        bytes.extend([0; 8]);
        if wide {
            bytes.extend((offset as u64).to_be_bytes());
            bytes.extend((slice.len() as u64).to_be_bytes());
            bytes.extend([0; 8]);
        } else {
            bytes.extend((offset as u32).to_be_bytes());
            bytes.extend((slice.len() as u32).to_be_bytes());
            bytes.extend([0; 4]);
        }
        offset += slice.len();
    }
    for slice in slices {
        bytes.extend(slice);
    }
    bytes
}
fn member(bytes: &mut Vec<u8>, name: &str, data: &[u8]) {
    bytes.extend(
        format!(
            "{name:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
            0,
            0,
            0,
            "644",
            data.len()
        )
        .as_bytes(),
    );
    bytes.extend(data);
    if !bytes.len().is_multiple_of(2) {
        bytes.push(b'\n');
    }
}
#[test]
fn thin_headers_support_both_endian_orders_and_cpu_families() {
    for (cpu, name, wide) in [
        (7, "i386", false),
        (0x1000007, "x86_64", true),
        (12, "arm", false),
        (0x100000c, "arm64", true),
        (18, "ppc", false),
        (0x1000012, "ppc64", true),
    ] {
        for little in [false, true] {
            assert_eq!(inspect(thin(cpu, little, wide)), [name]);
        }
    }
    assert!(inspect(b"ordinary text".to_vec()).is_empty());
    assert!(inspect(vec![]).is_empty());
}
#[test]
fn universal_and_static_archives_deduplicate_and_sort_architectures() {
    let arm = thin(0x100000c, true, true);
    let intel = thin(0x1000007, true, true);
    for wide in [false, true] {
        assert_eq!(
            inspect(fat(&[intel.clone(), arm.clone()], wide)),
            ["arm64", "x86_64"]
        );
    }
    let mut archive = b"!<arch>\n".to_vec();
    member(&mut archive, "/", b"symbol table");
    member(&mut archive, "a.o/", &arm);
    let mut named = b"longname.o".to_vec();
    named.extend(&intel);
    member(&mut archive, "#1/10", &named);
    member(&mut archive, "b.o/", &arm);
    assert_eq!(inspect(archive.clone()), ["arm64", "x86_64"]);
    // An extended name can place a nested archive at an odd absolute offset;
    // its member alignment is relative to that archive, not the outer file.
    let mut nested = b"!<arch>\n".to_vec();
    let mut payload = b"lib".to_vec();
    payload.extend(&archive);
    member(&mut nested, "#1/3", &payload);
    assert_eq!(inspect(nested), ["arm64", "x86_64"]);
    assert_eq!(inspect(fat(&[archive], false)), ["arm64", "x86_64"]);
}
#[test]
fn malformed_headers_commands_tables_and_members_are_rejected() {
    let mut truncated = thin(7, true, false);
    truncated.truncate(20);
    assert!(architectures(&mut Cursor::new(truncated)).is_err());
    let mut bad_commands = thin(7, true, false);
    bad_commands[16..20].copy_from_slice(&1u32.to_le_bytes());
    assert!(architectures(&mut Cursor::new(bad_commands)).is_err());
    let mut out_of_range = fat(&[thin(7, true, false)], true);
    out_of_range[16..24].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(architectures(&mut Cursor::new(out_of_range)).is_err());
    assert!(architectures(&mut Cursor::new(fat(&[b"plain data".to_vec()], false))).is_err());
    let mut archive = b"!<arch>\n".to_vec();
    member(&mut archive, "#1/99", b"short");
    assert!(architectures(&mut Cursor::new(archive)).is_err());
    let mut nested = thin(7, true, false);
    for _ in 0..34 {
        nested = fat(&[nested], false);
    }
    assert!(
        architectures(&mut Cursor::new(nested))
            .unwrap_err()
            .to_string()
            .contains("nesting")
    );
}
#[cfg(target_os = "macos")]
#[test]
fn inspects_real_cross_compiled_objects_and_static_library() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.c");
    std::fs::write(&source, "int answer(void) { return 42; }\n").unwrap();
    for (target, name) in [
        ("arm64-apple-macos11", "arm64"),
        ("x86_64-apple-macos11", "x86_64"),
    ] {
        let output = directory.path().join(format!("{name}.o"));
        let result = std::process::Command::new("/usr/bin/clang")
            .args(["-target", target, "-c"])
            .arg(&source)
            .arg("-o")
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            architectures(&mut std::fs::File::open(output).unwrap())
                .unwrap()
                .into_iter()
                .collect::<Vec<_>>(),
            [name]
        );
    }
    let archive = directory.path().join("library.a");
    let result = std::process::Command::new("/usr/bin/ar")
        .arg("rcs")
        .arg(&archive)
        .arg(directory.path().join("arm64.o"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        architectures(&mut std::fs::File::open(archive).unwrap())
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        ["arm64"]
    );
}

#[test]
fn registry_binary_probe_matches_macports_header_heuristics() {
    use port_rs::macho::is_binary;
    for magic in [0xfeedfaceu32, 0xfeedfacf] {
        assert!(is_binary(&mut &magic.to_ne_bytes()[..]).unwrap());
    }
    assert!(is_binary(&mut &b"!<arch>\n"[..]).unwrap());
    for count in [0u32, 1, 19, 20, 65_536] {
        let mut data = 0xcafebabeu32.to_be_bytes().to_vec();
        data.extend(count.to_be_bytes());
        assert_eq!(
            is_binary(&mut data.as_slice()).unwrap(),
            (1..20).contains(&count)
        );
    }
    for data in [
        b"".as_slice(),
        b"text",
        b"\xcf\xfa\xed",
        b"!<arch>",
        b"!<arNOPE",
        b"\xca\xfe\xba\xbe",
    ] {
        assert!(!is_binary(&mut &data[..]).unwrap());
    }
}

#[test]
fn registry_binary_probe_propagates_io_errors_and_reads_only_needed_header() {
    use std::io::{self, Read};
    struct HeaderThenError {
        bytes: &'static [u8],
    }
    impl Read for HeaderThenError {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if self.bytes.is_empty() {
                return Err(io::Error::other("read failed"));
            }
            self.bytes.read(output)
        }
    }
    assert!(
        port_rs::macho::is_binary(&mut HeaderThenError {
            bytes: b"\xca\xfe\xba\xbe"
        })
        .is_err()
    );
    assert!(!port_rs::macho::is_binary(&mut HeaderThenError { bytes: b"text" }).unwrap());
}

#[test]
fn linked_libraries_lists_dylib_install_names_of_a_real_binary() {
    let mut file = std::fs::File::open("/usr/bin/true").unwrap();
    let libraries = port_rs::macho::linked_libraries(&mut file).unwrap();
    assert!(
        libraries
            .iter()
            .any(|name| name.ends_with("libSystem.B.dylib")),
        "{libraries:?}"
    );
    let mut text = std::io::Cursor::new(b"#!/bin/sh\n".to_vec());
    assert!(
        port_rs::macho::linked_libraries(&mut text)
            .unwrap()
            .is_empty()
    );
}
