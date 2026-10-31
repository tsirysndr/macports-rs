//! `port sync` and `port selfupdate` (mportsync): refresh each ports tree
//! listed in sources.conf from its signed tarball and platform PortIndex.
use crate::{install::signature::TrustedKey, progress::Progress};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

struct Settings {
    portdbpath: PathBuf,
    sources: PathBuf,
    rsync_options: String,
    keys: Vec<TrustedKey>,
}

fn settings() -> Result<Settings> {
    let environment: BTreeMap<String, String> = std::env::vars().collect();
    // SAFETY: getuid has no arguments and does not mutate process state.
    let uid = unsafe { libc::getuid() };
    let paths = crate::config::Paths::discover(
        "/opt/local/etc/macports".into(),
        std::env::current_dir()?,
        &environment,
        uid,
    )?;
    let config = paths.load()?;
    let prefix = config.get("prefix").unwrap_or("/opt/local").to_string();
    let portdbpath = config
        .get("portdbpath")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(&prefix).join("var/macports"));
    let sources = config
        .get("sources_conf")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(&prefix).join("etc/macports/sources.conf"));
    let rsync_options = config
        .get("rsync_options")
        .unwrap_or("-rtzvl --delete-after")
        .to_string();
    let mut keys = vec![TrustedKey::macports_ports_key()?];
    if let Ok(entries) = fs::read_dir(Path::new(&prefix).join("share/macports/keys/ports")) {
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|ext| ext == "pub")
                && let Ok(text) = fs::read_to_string(entry.path())
                && let Ok(key) = TrustedKey::parse(&text)
                && !keys.iter().any(|known| known.id() == key.id())
            {
                keys.push(key);
            }
        }
    }
    Ok(Settings {
        portdbpath,
        sources,
        rsync_options,
        keys,
    })
}

/// `url [flags]` entries; `[nosync]` sources are skipped.
fn sources(path: &Path) -> Result<Vec<(String, Vec<String>)>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("cannot read sources {}", path.display()))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let url = words.next()?.to_string();
            let flags = words
                .flat_map(|word| word.trim_matches(['[', ']']).split(','))
                .map(|flag| flag.trim().to_string())
                .filter(|flag| !flag.is_empty())
                .collect();
            Some((url, flags))
        })
        .collect())
}

fn verify(path: &Path, keys: &[TrustedKey]) -> Result<()> {
    let signature = fs::read_to_string(format!("{}.sig", path.display()))
        .with_context(|| format!("missing signature for {}", path.display()))?;
    let file = fs::File::open(path)?;
    crate::install::signature::verify(file, &signature, keys)
        .with_context(|| format!("No known key verified signature for {}", path.display()))?;
    Ok(())
}

fn rsync(options: &str, includes: &[&str], source: &str, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    let mut command = Command::new("/usr/bin/rsync");
    command.args(options.split_whitespace());
    for include in includes {
        command.arg(format!("--include=/{include}"));
    }
    let status = command
        .stdout(std::process::Stdio::null())
        .arg("--exclude=*")
        .arg(source)
        .arg(destination)
        .status()
        .context("cannot run rsync")?;
    ensure!(status.success(), "rsync failed: {status}");
    Ok(())
}

fn download(url: &str, destination: &Path) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let bytes = runtime.block_on(async {
        let response = reqwest::get(url).await?.error_for_status()?;
        response.bytes().await
    })?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(destination, &bytes)?;
    Ok(())
}

/// Replace `<root>/ports` with the verified tarball's `ports` directory,
/// keeping the previous tree until extraction succeeds.
fn extract(tarball: &Path, root: &Path) -> Result<()> {
    let staging = root.join("ports.new");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    let mut file = fs::File::open(tarball)?;
    let mut magic = [0; 2];
    file.read_exact(&mut magic)?;
    let file = fs::File::open(tarball)?;
    let reader: Box<dyn Read> = if magic == [0x1f, 0x8b] {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut archive = tar::Archive::new(reader);
    archive.set_preserve_mtime(true);
    archive.set_overwrite(true);
    archive.unpack(&staging)?;
    let extracted = staging.join("ports");
    ensure!(
        extracted.is_dir(),
        "{} has no ports directory",
        tarball.display()
    );
    let target = root.join("ports");
    let old = root.join("ports.old");
    let _ = fs::remove_dir_all(&old);
    if target.exists() {
        fs::rename(&target, &old)?;
    }
    fs::rename(&extracted, &target)?;
    let _ = fs::remove_dir_all(&old);
    let _ = fs::remove_dir_all(&staging);
    Ok(())
}

/// Where a source's ports tree lives locally (upstream `getportdir`).
fn tree(url: &str, portdbpath: &Path) -> Option<PathBuf> {
    if let Some(path) = url.strip_prefix("file://") {
        return Some(PathBuf::from(path));
    }
    let (_, rest) = url.split_once("://")?;
    let rest = rest.trim_end_matches('/');
    let sources = portdbpath.join("sources");
    if rest.ends_with(".tar.gz") || rest.ends_with(".tar") {
        Some(sources.join(rest.rsplit_once('/')?.0).join("ports"))
    } else {
        Some(sources.join(rest))
    }
}

/// Local directories of every configured source's ports tree.
pub(super) fn source_trees() -> Vec<PathBuf> {
    let Ok(settings) = settings() else {
        return Vec::new();
    };
    sources(&settings.sources)
        .unwrap_or_default()
        .iter()
        .filter_map(|(url, _)| tree(url, &settings.portdbpath))
        .collect()
}

/// The PortIndex of the `[default]` source (or the first), once synced.
pub(super) fn default_index() -> Option<PathBuf> {
    let settings = settings().ok()?;
    let sources = sources(&settings.sources).ok()?;
    let (url, _) = sources
        .iter()
        .find(|(_, flags)| flags.iter().any(|flag| flag == "default"))
        .or(sources.first())?;
    Some(tree(url, &settings.portdbpath)?.join("PortIndex")).filter(|path| path.is_file())
}

fn sync_tarball(url: &str, settings: &Settings, index: &str) -> Result<()> {
    let (scheme, rest) = url.split_once("://").context("invalid source URL")?;
    let (base, filename) = rest.rsplit_once('/').context("invalid source URL")?;
    let root = tree(url, &settings.portdbpath)
        .and_then(|ports| ports.parent().map(Path::to_path_buf))
        .context("invalid source URL")?;
    let remote = root.join("remote");
    let tarball = remote.join(filename);
    let index_url = format!("{scheme}://{base}/{index}/");
    match scheme {
        "rsync" => {
            let files = [
                filename.to_string(),
                format!("{filename}.sig"),
                format!("{filename}.rmd160"),
            ];
            let includes: Vec<&str> = files.iter().map(String::as_str).collect();
            rsync(
                &settings.rsync_options,
                &includes,
                &format!("rsync://{base}/"),
                &remote,
            )?;
        }
        "http" | "https" => {
            download(url, &tarball)?;
            download(
                &format!("{url}.sig"),
                &remote.join(format!("{filename}.sig")),
            )?;
        }
        other => bail!("unsupported source protocol {other}"),
    }
    verify(&tarball, &settings.keys).context("Failed to verify signature for ports tree!")?;
    extract(&tarball, &root)?;
    // The platform-specific PortIndex is published beside the tarball.
    let index_remote = root.join("remote-index");
    match scheme {
        "rsync" => rsync(
            &settings.rsync_options,
            &["PortIndex", "PortIndex.sig", "PortIndex.rmd160"],
            &index_url,
            &index_remote,
        )?,
        _ => {
            download(
                &format!("{index_url}PortIndex"),
                &index_remote.join("PortIndex"),
            )?;
            download(
                &format!("{index_url}PortIndex.sig"),
                &index_remote.join("PortIndex.sig"),
            )?;
        }
    }
    let downloaded = index_remote.join("PortIndex");
    verify(&downloaded, &settings.keys).context("Failed to verify signature for PortIndex!")?;
    fs::copy(&downloaded, root.join("ports/PortIndex"))?;
    Ok(())
}

fn sync_directory(path: &Path, progress: &Progress) -> Result<()> {
    if path.join(".git").exists() {
        let status = Command::new("/usr/bin/git")
            .arg("-C")
            .arg(path)
            .args(["pull", "--rebase", "-q"])
            .status()
            .context("cannot run git")?;
        ensure!(status.success(), "Syncing local git ports tree failed");
    }
    if !path.join("PortIndex").is_file() {
        progress.warning(&format!(
            "{} has no PortIndex; generating one is not supported yet",
            path.display()
        ));
    }
    Ok(())
}

pub(super) fn run(quiet: bool, selfupdate: bool) -> Result<()> {
    let progress = Progress::new(quiet);
    let settings = settings()?;
    if selfupdate {
        progress.message(
            "--->  port-rs is updated with its own release process; MacPorts base is not replaced",
        );
    }
    progress.message("--->  Updating the ports tree");
    let index = crate::source::detect_platform()?.directory();
    let mut failed = 0;
    for (url, flags) in sources(&settings.sources)? {
        if flags.iter().any(|flag| flag == "nosync") {
            continue;
        }
        let result = if let Some(path) = url.strip_prefix("file://") {
            sync_directory(Path::new(path), &progress)
        } else {
            sync_tarball(&url, &settings, &index)
        };
        if let Err(err) = result {
            progress.message(&format!("Error: Synchronization of {url} failed: {err:#}"));
            failed += 1;
        }
    }
    ensure!(failed == 0, "Synchronization of {failed} source(s) failed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_map_to_upstream_tree_locations() {
        let db = Path::new("/db");
        assert_eq!(
            tree(
                "rsync://rsync.macports.org/macports/release/tarballs/ports.tar.gz",
                db
            ),
            Some(PathBuf::from(
                "/db/sources/rsync.macports.org/macports/release/tarballs/ports"
            ))
        );
        assert_eq!(
            tree("rsync://example.org/ports/", db),
            Some(PathBuf::from("/db/sources/example.org/ports"))
        );
        assert_eq!(
            tree("file:///src/ports", db),
            Some(PathBuf::from("/src/ports"))
        );
    }

    #[test]
    fn unsigned_or_tampered_files_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ports.tar.gz");
        fs::write(&file, b"data").unwrap();
        let keys = [TrustedKey::macports_ports_key().unwrap()];
        assert!(verify(&file, &keys).is_err());
        let synced = Path::new(
            "/opt/local/var/macports/sources/rsync.macports.org/macports/release/tarballs/remote/PortIndex.sig",
        );
        if let Ok(signature) = fs::read_to_string(synced) {
            fs::write(dir.path().join("ports.tar.gz.sig"), signature).unwrap();
            assert!(verify(&file, &keys).is_err());
        }
    }
}
