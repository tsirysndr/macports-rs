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
    load(config, cwd, site_keys, &[])
}

/// Read selected local site-key paths and pubkeys.conf in one deduplicated pass.
/// Site keys are selected by signature type, so their filenames need not end in .pub.
pub fn load_signify_paths(config: &Path, cwd: &Path, site_paths: &[PathBuf]) -> Result<Trust> {
    load(config, cwd, Vec::new(), site_paths)
}

fn load(
    config: &Path,
    cwd: &Path,
    site_keys: Vec<TrustedKey>,
    site_paths: &[PathBuf],
) -> Result<Trust> {
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
    let mut paths = HashSet::new();
    for path in site_paths {
        load_key(absolute(cwd, path), &mut paths, &mut trust)?;
    }
    let metadata = match std::fs::metadata(&config) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => return Ok(trust),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(trust),
        Err(error) => return Err(error).context("cannot inspect pubkeys.conf"),
    };
    ensure!(metadata.len() <= 1024 * 1024, "pubkeys.conf exceeds 1 MiB");
    let source = read_bounded(&config, 1024 * 1024)?;
    for line in source.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let path = absolute(cwd, Path::new(line));
        if path.extension().is_none_or(|extension| extension != "pub") {
            continue;
        }
        load_key(path, &mut paths, &mut trust)?;
    }
    Ok(trust)
}
fn load_key(path: PathBuf, paths: &mut HashSet<PathBuf>, trust: &mut Trust) -> Result<()> {
    if !paths.insert(path.clone()) {
        return Ok(());
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
    Ok(())
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
