//! Archive sites signed with RSA (`.rmd160`), verified with `openssl dgst
//! -ripemd160` against `.pem` keys as upstream does.
use super::{
    Selection,
    requests::{Subdirectories, archive_url, resolve_mirror},
};
use crate::install::{Item, target::Target};
use anyhow::{Result, ensure};
use std::path::{Path, PathBuf};

/// Archive URLs from the references whose sites use RSA signatures, and the
/// sites' own `.pem` keys.
pub struct LegacyRequest {
    pub urls: Vec<String>,
    pub keys: Vec<PathBuf>,
}

impl Selection {
    pub fn legacy_request(
        &self,
        session: &mut crate::portfile::Session,
        target: &Target,
        item: &Item<'_>,
    ) -> Result<LegacyRequest> {
        let references = if session.invoke("exists", &["archive_sites"])? == "1" {
            crate::tcl_list::parse(&session.scalar("archive_sites")?)?
        } else {
            self.defaults.clone()
        };
        let name = session.scalar("name")?;
        let archive = session.scalar("archive.subdir")?;
        let dist = session.scalar("dist_subdir")?;
        let mirror = if session.invoke("exists", &["archive_sites.mirror_subdir"])? == "1" {
            session.scalar("archive_sites.mirror_subdir")?
        } else {
            String::new()
        };
        let filename = target.filename(item)?;
        let mut urls = Vec::new();
        let mut keys = Vec::new();
        for reference in &references {
            let parts: Vec<_> = reference.split(':').collect();
            let Some(site) = self.get(parts[0]) else {
                continue;
            };
            if site.signature_type != "rmd160" || site.archive_type != "tbz2" {
                continue;
            }
            let subdir = format!("{}{mirror}", parts.get(1).copied().unwrap_or(""));
            let subdirs = Subdirectories {
                port_name: &name,
                mirror: &subdir,
                archive: &archive,
                dist: &dist,
            };
            for element in &site.urls {
                let base = resolve_mirror(element, subdirs.mirror, subdirs.port_name, subdirs.dist);
                urls.push(archive_url(&base, subdirs.archive, &filename)?);
            }
            if let Some(key) = site.public_key.as_ref().filter(|key| key.ends_with(".pem")) {
                keys.push(PathBuf::from(key));
            }
        }
        ensure!(
            !urls.is_empty(),
            "{} has no RSA-signed archive sites",
            item.port.name
        );
        Ok(LegacyRequest { urls, keys })
    }
}

/// `.pem` keys listed in pubkeys.conf, relative entries against `cwd`.
pub fn configured_pem_keys(config: &Path, cwd: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(config)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#') && line.ends_with(".pem"))
        .map(|line| cwd.join(line))
        .collect()
}

/// DER DigestInfo prefix for a RIPEMD-160 digest (OID 1.3.36.3.2.1).
const RIPEMD160_PREFIX: [u8; 15] = [
    0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x24, 0x03, 0x02, 0x01, 0x05, 0x00, 0x04, 0x14,
];

fn public_key(pem: &str) -> Option<rsa::RsaPublicKey> {
    use rsa::{pkcs1::DecodeRsaPublicKey, pkcs8::DecodePublicKey};
    rsa::RsaPublicKey::from_public_key_pem(pem)
        .ok()
        .or_else(|| rsa::RsaPublicKey::from_pkcs1_pem(pem).ok())
}

/// Whether `signature` (PKCS #1 v1.5 RSA over RIPEMD-160, as
/// `openssl dgst -ripemd160 -sign` writes) verifies `file` with any key.
pub fn verify(file: &Path, signature: &Path, keys: &[PathBuf]) -> bool {
    use ripemd::Digest;
    let (Ok(contents), Ok(signature)) = (std::fs::read(file), std::fs::read(signature)) else {
        return false;
    };
    let digest = ripemd::Ripemd160::digest(&contents);
    keys.iter().any(|key| {
        let Some(key) = std::fs::read_to_string(key)
            .ok()
            .and_then(|pem| public_key(&pem))
        else {
            return false;
        };
        let scheme = rsa::Pkcs1v15Sign {
            hash_len: Some(20),
            prefix: RIPEMD160_PREFIX.to_vec().into_boxed_slice(),
        };
        key.verify(scheme, &digest, &signature).is_ok()
    })
}
