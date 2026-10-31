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

/// Whether `signature` (DER RSA over RIPEMD-160) verifies `file` with any key.
pub fn verify(file: &Path, signature: &Path, keys: &[PathBuf]) -> bool {
    keys.iter().any(|key| {
        std::process::Command::new("/usr/bin/openssl")
            .args(["dgst", "-ripemd160", "-verify"])
            .arg(key)
            .arg("-signature")
            .arg(signature)
            .arg(file)
            .output()
            .is_ok_and(|output| output.status.success())
    })
}
