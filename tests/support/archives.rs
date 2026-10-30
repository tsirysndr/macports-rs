#![allow(dead_code)]
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_compact::{KeyPair, Seed};
use port_rs::install::{
    archive::VerifiedArchive,
    inspect::{self, Limits},
    signature::TrustedKey,
};
use std::io::{Cursor, Write};

pub fn contents(files: &[&str]) -> String {
    format!(
        "@name app-1_0+ssl\n@portname app\n@portversion 1\n@portrevision 0\n@portepoch 0\n@archs arm64\n@portvariant +ssl\n@pkgdep base-2_0\n{}\n",
        files.join("\n")
    )
}
pub fn tar(records: &[(&str, u8, &[u8], &str)]) -> Vec<u8> {
    tar_with_owner(records, 501, 20)
}
pub fn tar_with_owner(records: &[(&str, u8, &[u8], &str)], uid: u64, gid: u64) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, kind, data, link) in records {
        let mut header = tar::Header::new_ustar();
        // Set raw bytes to exercise paths a safe producer would normally reject.
        header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
        header.set_entry_type(tar::EntryType::new(*kind));
        header.set_size(data.len() as u64);
        header.set_mode(0o755);
        header.set_uid(uid);
        header.set_gid(gid);
        header.set_mtime(1_700_000_000);
        if !link.is_empty() {
            header.set_link_name(link).unwrap();
        }
        header.set_cksum();
        builder.append(&header, *data).unwrap();
    }
    builder.into_inner().unwrap()
}
pub fn compress(bytes: &[u8]) -> Vec<u8> {
    let mut writer = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
    writer.write_all(bytes).unwrap();
    writer.finish().unwrap()
}
pub fn stage(bytes: &[u8]) -> VerifiedArchive {
    let key = KeyPair::from_seed(Seed::new([1; 32]));
    let packet = |data: &[u8]| {
        let mut packet = b"Ed12345678".to_vec();
        packet.extend(data);
        format!("untrusted comment: fixture\n{}\n", STANDARD.encode(packet))
    };
    let public = TrustedKey::parse(&packet(key.pk.as_ref())).unwrap();
    let temp = tempfile::tempdir().unwrap();
    VerifiedArchive::stage(
        Cursor::new(bytes),
        &packet(key.sk.sign(bytes, None).as_ref()),
        &[public],
        temp.path(),
        1024 * 1024,
    )
    .unwrap()
}
pub fn trusted_key() -> TrustedKey {
    let key = KeyPair::from_seed(Seed::new([1; 32]));
    let mut packet = b"Ed12345678".to_vec();
    packet.extend(key.pk.as_ref());
    TrustedKey::parse(&format!(
        "untrusted comment: fixture\n{}\n",
        STANDARD.encode(packet)
    ))
    .unwrap()
}
pub fn inspect_records(records: &[(&str, u8, &[u8], &str)]) -> anyhow::Result<inspect::Inspection> {
    inspect::tbz2(&mut stage(&compress(&tar(records))), Limits::default())
}
