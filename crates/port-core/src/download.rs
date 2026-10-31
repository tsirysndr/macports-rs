use crate::progress::{Progress, TransferProgress};
use anyhow::{Context, Result, bail, ensure};
use futures_util::{StreamExt, stream};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub files: Vec<Download>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Download {
    pub name: String,
    pub urls: Vec<String>,
    pub sha256: String,
    pub size: Option<u64>,
}
#[derive(Debug)]
pub struct Outcome {
    pub path: PathBuf,
    pub cached: bool,
}

pub fn validate(manifest: &Manifest) -> Result<()> {
    let mut names = HashSet::new();
    for file in &manifest.files {
        let mut components = Path::new(&file.name).components();
        ensure!(
            matches!(components.next(), Some(Component::Normal(_)))
                && components.next().is_none()
                && !file.name.contains(['/', '\\']),
            "invalid download filename {:?}",
            file.name
        );
        ensure!(
            names.insert(file.name.to_lowercase()),
            "duplicate download filename {}",
            file.name
        );
        ensure!(
            file.sha256.len() == 64 && file.sha256.bytes().all(|c| c.is_ascii_hexdigit()),
            "invalid SHA-256 for {}",
            file.name
        );
        ensure!(!file.urls.is_empty(), "no URLs for {}", file.name);
        for url in &file.urls {
            let url = reqwest::Url::parse(url)?;
            ensure!(
                matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
                "only HTTP(S) download URLs are supported"
            );
        }
    }
    Ok(())
}

pub async fn run(manifest: &Manifest, directory: &Path, jobs: usize) -> Result<Vec<Outcome>> {
    run_with_progress(manifest, directory, jobs, &Progress::new(true)).await
}

pub async fn run_with_progress(
    manifest: &Manifest,
    directory: &Path,
    jobs: usize,
    progress: &Progress,
) -> Result<Vec<Outcome>> {
    ensure!((1..=128).contains(&jobs), "jobs must be between 1 and 128");
    validate(manifest)?;
    progress.phase(&format!(
        "Fetching {} files ({jobs} concurrent transfers)",
        manifest.files.len()
    ));
    tokio::fs::create_dir_all(directory).await?;
    let client = reqwest::Client::builder()
        .user_agent(concat!("port-rs/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .timeout(Duration::from_secs(3600))
        .build()?;
    // Drain all futures even on failure so temporary files and other transfers finish cleanly.
    let results: Vec<Result<Outcome>> = stream::iter(
        manifest
            .files
            .iter()
            .map(|file| download(&client, file, directory, progress)),
    )
    .buffer_unordered(jobs)
    .collect()
    .await;
    let mut outcomes = Vec::new();
    let mut errors = Vec::new();
    for result in results {
        match result {
            Ok(o) => outcomes.push(o),
            Err(e) => errors.push(format!("{e:#}")),
        }
    }
    ensure!(
        errors.is_empty(),
        "downloads failed:\n{}",
        errors.join("\n")
    );
    progress.phase("All downloads verified");
    outcomes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(outcomes)
}

async fn verified(path: &Path, file: &Download) -> Result<bool> {
    let mut input = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut size = 0u64;
    loop {
        let n = input.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        size += n as u64;
    }
    Ok(file.size.is_none_or(|s| s == size)
        && format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(&file.sha256))
}

async fn download(
    client: &reqwest::Client,
    file: &Download,
    directory: &Path,
    progress: &Progress,
) -> Result<Outcome> {
    let bar = progress.file(&file.name);
    let destination = directory.join(&file.name);
    match tokio::fs::symlink_metadata(&destination).await {
        Ok(metadata) => {
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "{} is not a regular file",
                destination.display()
            );
            ensure!(
                verified(&destination, file).await?,
                "existing file {} failed verification; remove it to retry",
                destination.display()
            );
            bar.verified();
            return Ok(Outcome {
                path: destination,
                cached: true,
            });
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    let mut errors = Vec::new();
    for url in &file.urls {
        for attempt in 0..3 {
            match transfer(client, url, file, directory, &destination, &bar).await {
                Ok(()) => {
                    bar.verified();
                    return Ok(Outcome {
                        path: destination,
                        cached: false,
                    });
                }
                Err(e) => errors.push(format!("{url}: {e:#}")),
            }
            if attempt < 2 {
                tokio::time::sleep(Duration::from_millis(100 * (1 << attempt))).await;
            }
        }
    }
    bail!("{}: {}", file.name, errors.join("; "))
}

async fn transfer(
    client: &reqwest::Client,
    url: &str,
    file: &Download,
    directory: &Path,
    destination: &Path,
    bar: &TransferProgress,
) -> Result<()> {
    let response = client.get(url).send().await?.error_for_status()?;
    bar.start(file.size.or(response.content_length()));
    let temporary = tempfile::Builder::new()
        .prefix(".port-rs-")
        .tempfile_in(directory)?;
    let mut output = tokio::fs::File::from_std(temporary.as_file().try_clone()?);
    let mut body = response.bytes_stream();
    let mut hash = Sha256::new();
    let mut size = 0u64;
    while let Some(chunk) = body.next().await {
        let chunk = chunk?;
        size += chunk.len() as u64;
        ensure!(
            file.size.is_none_or(|expected| size <= expected),
            "download exceeds expected size"
        );
        hash.update(&chunk);
        output.write_all(&chunk).await?;
        bar.advance(chunk.len() as u64);
    }
    bar.message("verifying SHA-256");
    ensure!(
        file.size.is_none_or(|expected| size == expected),
        "download size mismatch"
    );
    ensure!(
        format!("{:x}", hash.finalize()).eq_ignore_ascii_case(&file.sha256),
        "SHA-256 mismatch"
    );
    output.flush().await?;
    output.sync_all().await?;
    drop(output);
    temporary
        .persist_noclobber(destination)
        .with_context(|| format!("cannot publish {}", destination.display()))?;
    Ok(())
}
