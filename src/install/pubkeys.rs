//! Configured Signify trust keys for archive verification. Never read from mirrors.
use super::signature::TrustedKey;
use anyhow::{Context, Result, ensure};
use std::{
    collections::HashSet,
    fs::OpenOptions,
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

pub struct Trust {
    pub keys: Vec<TrustedKey>,
    pub warnings: Vec<String>,
}
/// Add pubkeys.conf entries to explicitly supplied archive-site trust keys.
/// Legacy .pem entries apply to .rmd160 signatures and are not selected here.
/// Relative entries are relative to startup cwd, not the configuration directory.
pub fn load_signify(config: &Path, cwd: &Path, site_keys: Vec<TrustedKey>) -> Result<Trust> {
    ensure!(
        cwd.is_absolute(),
        "startup working directory must be absolute"
    );
    ensure!(site_keys.len() <= 64, "too many archive-site trust keys");
    let config = absolute(cwd, config);
    let mut trust = Trust {
        keys: site_keys,
        warnings: Vec::new(),
    };
    let metadata = match std::fs::metadata(&config) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => return Ok(trust),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(trust),
        Err(error) => return Err(error).context("cannot inspect pubkeys.conf"),
    };
    ensure!(metadata.len() <= 1024 * 1024, "pubkeys.conf exceeds 1 MiB");
    let source = read_bounded(&config, 1024 * 1024)?;
    let mut paths = HashSet::new();
    for line in source.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let path = absolute(cwd, Path::new(line));
        if path.extension().is_none_or(|extension| extension != "pub")
            || !paths.insert(path.clone())
        {
            continue;
        }
        let key = read_bounded(&path, 4096).and_then(|text| TrustedKey::parse(&text));
        match key {
            Ok(key) => {
                ensure!(trust.keys.len() < 64, "too many Signify trust keys");
                trust.keys.push(key);
            }
            Err(error) => trust.warnings.push(format!(
                "cannot load archive public key {}: {error:#}",
                path.display()
            )),
        }
    }
    Ok(trust)
}
fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        cwd.join(path)
    }
}
fn read_bounded(path: &Path, limit: u64) -> Result<String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    ensure!(
        file.metadata()?.is_file(),
        "public-key input is not a regular file"
    );
    let mut text = String::new();
    file.take(limit + 1).read_to_string(&mut text)?;
    ensure!(
        text.len() as u64 <= limit,
        "public-key input exceeds size limit"
    );
    Ok(text)
}
