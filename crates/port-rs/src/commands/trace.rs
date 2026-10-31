//! Trace mode's report (upstream porttrace): darwintrace records the prefix
//! paths a target's programs touch, and after the target the paths are
//! sorted into files trace mode hid, files it would have hidden had they
//! existed, and files no port installed.
use crate::{progress::Progress, registry::Registry};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashSet},
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

const DARWINTRACE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/darwintrace.dylib"));

pub(super) struct Tracer {
    pub log: PathBuf,
    dylib: PathBuf,
    prefix: String,
    pub sipdir: PathBuf,
    /// Paths that belong to the build, not to the prefix's ports.
    ignored: Vec<String>,
}

fn sip_protected(path: &Path) -> bool {
    [
        "/bin/",
        "/sbin/",
        "/usr/bin/",
        "/usr/sbin/",
        "/usr/libexec/",
        "/System/",
    ]
    .iter()
    .any(|root| path.to_string_lossy().starts_with(root))
}

/// arm64e slices cannot carry an ad-hoc signature; mark them plain arm64,
/// as upstream's `sip_copy_proc` does.
fn patch_arm64e(bytes: &mut [u8]) {
    const CPU_TYPE_ARM64: u32 = 0x0100_000c;
    let read = |bytes: &[u8], at: usize, big: bool| -> Option<u32> {
        let word: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
        Some(if big {
            u32::from_be_bytes(word)
        } else {
            u32::from_le_bytes(word)
        })
    };
    match read(bytes, 0, true) {
        Some(0xcafe_babe) => {
            let count = read(bytes, 4, true).unwrap_or(0).min(64) as usize;
            for index in 0..count {
                let at = 8 + index * 20;
                let (Some(cputype), Some(subtype), Some(offset)) = (
                    read(bytes, at, true),
                    read(bytes, at + 4, true),
                    read(bytes, at + 8, true),
                ) else {
                    return;
                };
                if cputype == CPU_TYPE_ARM64 && subtype & 0x00ff_ffff == 2 {
                    bytes[at + 4..at + 8].copy_from_slice(&0u32.to_be_bytes());
                    let header = offset as usize + 8;
                    if let Some(field) = bytes.get_mut(header..header + 4) {
                        field.copy_from_slice(&0u32.to_le_bytes());
                    }
                }
            }
        }
        _ if read(bytes, 0, false) == Some(0xfeed_facf)
            && read(bytes, 4, false) == Some(CPU_TYPE_ARM64)
            && read(bytes, 8, false).is_some_and(|subtype| subtype & 0x00ff_ffff == 2) =>
        {
            bytes[8..12].copy_from_slice(&0u32.to_le_bytes());
        }
        _ => {}
    }
}

/// A traceable copy of a SIP-protected program under `sipdir`.
pub(super) fn sip_copy(program: &Path, sipdir: &Path) -> Result<PathBuf> {
    let copy = sipdir.join(program.strip_prefix("/").unwrap_or(program));
    let original = fs::metadata(program)?;
    if let Ok(existing) = fs::metadata(&copy)
        && existing.len() == original.len()
        && existing.modified()? == original.modified()?
    {
        return Ok(copy);
    }
    let directory = copy.parent().context("SIP copy has no directory")?;
    fs::create_dir_all(directory)?;
    let staged = tempfile::Builder::new()
        .prefix(".sip-copy.")
        .tempfile_in(directory)?;
    let mut bytes = fs::read(program)?;
    patch_arm64e(&mut bytes);
    fs::write(staged.path(), &bytes)?;
    let status = Command::new("/usr/bin/codesign")
        .args(["-s", "-", "-f"])
        .arg(staged.path())
        .env_clear()
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if !status.success() {
        bail!("cannot re-sign a copy of {}", program.display());
    }
    fs::set_permissions(
        staged.path(),
        fs::Permissions::from_mode(original.permissions().mode() & 0o755),
    )?;
    let file = fs::File::options().write(true).open(staged.path())?;
    file.set_modified(original.modified()?)?;
    staged.persist(&copy)?;
    Ok(copy)
}

impl Tracer {
    /// Install darwintrace and choose the log for one target.
    pub(super) fn new(
        portdbpath: &Path,
        prefix: &str,
        workpath: &Path,
        target: &str,
        ignored: Vec<String>,
    ) -> Result<Option<Self>> {
        if DARWINTRACE.is_empty() {
            return Ok(None);
        }
        let digest: String = Sha256::digest(DARWINTRACE)
            .iter()
            .take(8)
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let dylib = portdbpath
            .join("darwintrace")
            .join(format!("darwintrace-{digest}.dylib"));
        if !dylib.is_file() {
            fs::create_dir_all(dylib.parent().unwrap())?;
            let staged = tempfile::NamedTempFile::new_in(dylib.parent().unwrap())?;
            fs::write(staged.path(), DARWINTRACE)?;
            fs::set_permissions(staged.path(), fs::Permissions::from_mode(0o644))?;
            staged.persist(&dylib)?;
        }
        fs::create_dir_all(workpath)?;
        let log = workpath.join(format!(".darwintrace-{target}.log"));
        let _ = fs::remove_file(&log);
        Ok(Some(Self {
            log,
            dylib,
            prefix: prefix.trim_end_matches('/').to_owned(),
            sipdir: portdbpath.join("sip-workaround"),
            ignored,
        }))
    }

    /// Run `command` traced: its program (a SIP copy when protected) starts
    /// through `env` with darwintrace's variables, so an enclosing
    /// sandbox-exec cannot strip them.
    pub(super) fn wrap(&self, command: &mut crate::build::Command) -> Result<()> {
        let program = PathBuf::from(&command.program);
        let program = if sip_protected(&program) {
            sip_copy(&program, &self.sipdir)?
        } else {
            program
        };
        let mut args: Vec<std::ffi::OsString> = vec![
            format!("DYLD_INSERT_LIBRARIES={}", self.dylib.display()).into(),
            format!("DARWINTRACE_LOG={}", self.log.display()).into(),
            format!("DARWINTRACE_PREFIX={}", self.prefix).into(),
            format!("DARWINTRACE_SIPDIR={}", self.sipdir.display()).into(),
            program.into(),
        ];
        args.append(&mut command.args);
        command.args = args;
        command.program = PathBuf::from("/usr/bin/env").into();
        Ok(())
    }

    /// Print upstream's trace report for the target, then drop the log.
    pub(super) fn report(
        &self,
        hidden: &HashSet<String>,
        registry_path: &Path,
        progress: &Progress,
    ) {
        let Ok(text) = fs::read_to_string(&self.log) else {
            return;
        };
        let _ = fs::remove_file(&self.log);
        let paths: BTreeSet<&str> = text.lines().collect();
        let registry = Registry::open(Some(registry_path)).ok().flatten();
        let (mut existing, mut missing, mut unknown) = (Vec::new(), Vec::new(), Vec::new());
        for path in paths {
            if path == self.prefix
                || self.ignored.iter().any(|directory| {
                    path == directory || path.starts_with(&format!("{directory}/"))
                })
            {
                continue;
            }
            let metadata = fs::symlink_metadata(path);
            if hidden.contains(path) {
                if metadata.is_ok() {
                    existing.push(path);
                } else {
                    missing.push(path);
                }
                continue;
            }
            match metadata {
                Err(_) => missing.push(path),
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => {
                    let owned = registry
                        .as_ref()
                        .and_then(|registry| registry.owner(path, true).ok().flatten())
                        .is_some();
                    if !owned {
                        unknown.push(path);
                    }
                }
            }
        }
        let plural = |count: usize, one: &str, many: &str| {
            if count == 1 {
                one.to_owned()
            } else {
                many.to_owned()
            }
        };
        if !existing.is_empty() {
            progress.warning(&plural(
                existing.len(),
                "The following existing file was hidden from the build system by trace mode:",
                "The following existing files were hidden from the build system by trace mode:",
            ));
            for path in &existing {
                progress.message(&format!("  {path}"));
            }
        }
        if !missing.is_empty() {
            progress.log(
                "info",
                &plural(
                    missing.len(),
                    "The following file would have been hidden from the build system by trace mode if it existed:",
                    "The following files would have been hidden from the build system by trace mode if they existed:",
                ),
            );
            for path in &missing {
                progress.log("info", &format!("  {path}"));
            }
        }
        if !unknown.is_empty() {
            progress.warning(&plural(
                unknown.len(),
                "The following file inside the MacPorts prefix not installed by a port was accessed:",
                "The following files inside the MacPorts prefix not installed by a port were accessed:",
            ));
            for path in &unknown {
                progress.message(&format!("  {path}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arm64e_slices_become_plain_arm64() {
        let mut fat = vec![0u8; 0x2000];
        fat[0..4].copy_from_slice(&0xcafe_babeu32.to_be_bytes());
        fat[4..8].copy_from_slice(&1u32.to_be_bytes());
        fat[8..12].copy_from_slice(&0x0100_000cu32.to_be_bytes());
        fat[12..16].copy_from_slice(&0x8000_0002u32.to_be_bytes());
        fat[16..20].copy_from_slice(&0x1000u32.to_be_bytes());
        fat[0x1000..0x1004].copy_from_slice(&0xfeed_facfu32.to_le_bytes());
        fat[0x1008..0x100c].copy_from_slice(&0x8000_0002u32.to_le_bytes());
        patch_arm64e(&mut fat);
        assert_eq!(fat[12..16], 0u32.to_be_bytes());
        assert_eq!(fat[0x1008..0x100c], 0u32.to_le_bytes());
    }
}
