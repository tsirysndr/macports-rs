//! The base half of `port selfupdate`: replace this `port` executable with a
//! newer published release, as MacPorts base updates itself.
//!
//! Releases are read from the project's GitHub releases (or
//! `PORT_RS_RELEASES_URL`). An asset named `port-<target>.tar.gz` holds the
//! `port` binary; a matching `.sha256` asset, when published, is verified.
use crate::progress::Progress;
use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};

const RELEASES: &str = "https://api.github.com/repos/tsirysndr/macports-rs/releases/latest";

/// The native target of this Mac, even when running under Rosetta.
fn target() -> &'static str {
    if cfg!(target_arch = "aarch64") || crate::source::translated() {
        "aarch64-apple-darwin"
    } else {
        "x86_64-apple-darwin"
    }
}

fn get(url: &str) -> Result<Vec<u8>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let response = reqwest::Client::builder()
            .user_agent(format!("port-rs/{}", env!("CARGO_PKG_VERSION")))
            .build()?
            .get(url)
            .send()
            .await?;
        ensure!(
            response.status().is_success(),
            "{url}: HTTP {}",
            response.status().as_u16()
        );
        Ok(response.bytes().await?.to_vec())
    })
}

/// The newest release: its version and the asset URLs by name.
fn latest() -> Result<(String, Vec<(String, String)>)> {
    let url = std::env::var("PORT_RS_RELEASES_URL").unwrap_or_else(|_| RELEASES.into());
    let release: serde_json::Value =
        serde_json::from_slice(&get(&url)?).context("invalid release metadata")?;
    let tag = release["tag_name"]
        .as_str()
        .context("release metadata has no tag_name")?;
    let assets = release["assets"]
        .as_array()
        .map(|assets| {
            assets
                .iter()
                .filter_map(|asset| {
                    Some((
                        asset["name"].as_str()?.to_string(),
                        asset["browser_download_url"].as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok((tag.trim_start_matches('v').to_string(), assets))
}

/// The `port` binary inside a release tarball.
fn extract(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry.path()?.file_name().is_some_and(|name| name == "port") {
            let mut binary = Vec::new();
            entry.read_to_end(&mut binary)?;
            return Ok(binary);
        }
    }
    bail!("the release archive has no port executable")
}

/// Replace `executable` atomically, keeping its permissions.
fn replace(executable: &Path, binary: &[u8]) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let directory = executable
        .parent()
        .context("the port executable has no directory")?;
    let mut staged = tempfile::Builder::new()
        .prefix(".port-update.")
        .tempfile_in(directory)
        .with_context(|| {
            format!(
                "cannot write to {}; try again with sudo",
                directory.display()
            )
        })?;
    std::io::Write::write_all(&mut staged, binary)?;
    let mode = std::fs::metadata(executable)
        .map(|metadata| metadata.permissions().mode())
        .unwrap_or(0o755);
    std::fs::set_permissions(staged.path(), std::fs::Permissions::from_mode(mode | 0o111))?;
    staged
        .persist(executable)
        .map_err(|error| error.error)
        .with_context(|| format!("cannot replace {}", executable.display()))?;
    Ok(())
}

/// Update this executable when a newer release exists, or reinstall the
/// latest one when `reinstall` is set; returns whether it did.
pub(super) fn update(progress: &Progress, reinstall: bool) -> Result<bool> {
    progress.phase("Updating port-rs base");
    let current = env!("CARGO_PKG_VERSION");
    let (version, assets) = match latest() {
        Ok(release) => release,
        Err(error) if error.to_string().contains("HTTP 404") => {
            progress.message("No port-rs releases are published; port-rs base was not changed");
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    let order = crate::registry::compare_versions(&version, current);
    if order.is_lt() || (order.is_eq() && !reinstall) {
        progress.message(&format!(
            "port-rs base is already the latest version ({current})"
        ));
        return Ok(false);
    }
    let name = format!("port-{}.tar.gz", target());
    let Some((_, url)) = assets.iter().find(|(asset, _)| *asset == name) else {
        progress.message(&format!(
            "port-rs {version} is available, but has no prebuilt {} binary; build it with cargo install --git https://github.com/tsirysndr/macports-rs port-rs",
            target()
        ));
        return Ok(false);
    };
    let archive = get(url)?;
    if let Some((_, url)) = assets
        .iter()
        .find(|(asset, _)| *asset == format!("{name}.sha256"))
    {
        let expected = String::from_utf8_lossy(&get(url)?)
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let actual: String = Sha256::digest(&archive)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        ensure!(actual == expected, "checksum mismatch for {name}");
    }
    let binary = extract(&archive)?;
    let executable = std::env::current_exe()?;
    replace(&executable, &binary)?;
    progress.message(&format!(
        "port-rs base has been updated from {current} to {version}"
    ));
    Ok(true)
}
