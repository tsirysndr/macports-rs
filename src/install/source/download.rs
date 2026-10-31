use super::Request;
use crate::progress::{Progress, TransferProgress};
use anyhow::{Context, Result, bail, ensure};
use futures_util::StreamExt;
use reqwest::{Client, Url};
use std::{io::Write, os::unix::fs::PermissionsExt, path::Path, time::Duration};

/// Fetch missing distfiles sequentially, in candidate order, like upstream.
pub async fn run(
    requests: &[Request],
    progress: &Progress,
    proxies: Option<&crate::install::proxy::Resolved>,
) -> Result<usize> {
    let builder = Client::builder()
        .user_agent(concat!("port-rs/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60));
    let client = match proxies {
        Some(proxies) => proxies.configure_client(builder)?,
        None => builder,
    }
    .build()?;
    let mut fetched = 0;
    for request in requests {
        if request.destination.is_file() {
            continue;
        }
        fetch(&client, request, progress).await?;
        fetched += 1;
    }
    Ok(fetched)
}

async fn fetch(client: &Client, request: &Request, progress: &Progress) -> Result<()> {
    let directory = request
        .destination
        .parent()
        .context("distfile destination has no directory")?;
    let transfer = progress.file(&request.filename);
    let mut errors = Vec::new();
    for url in &request.urls {
        match attempt(client, url, directory, request, &transfer).await {
            Ok(()) => {
                transfer.verified();
                return Ok(());
            }
            Err(error) => errors.push(format!("{url}: {error:#}")),
        }
    }
    if errors.is_empty() {
        bail!("no fetch sites for {}", request.filename);
    }
    bail!(
        "fetch of {} failed: {}",
        request.filename,
        errors.join("; ")
    )
}

async fn attempt(
    client: &Client,
    url: &str,
    directory: &Path,
    request: &Request,
    transfer: &TransferProgress,
) -> Result<()> {
    let url = Url::parse(url)?;
    let mut file = tempfile::Builder::new()
        .prefix(&format!("{}.TMP", request.filename))
        .tempfile_in(directory)?;
    match url.scheme() {
        "file" => {
            let source = url
                .to_file_path()
                .map_err(|()| anyhow::anyhow!("invalid file URL"))?;
            ensure!(source.is_file(), "no such file");
            let mut input = std::fs::File::open(&source)?;
            transfer.start(input.metadata().ok().map(|data| data.len()));
            let copied = std::io::copy(&mut input, file.as_file_mut())?;
            transfer.advance(copied);
        }
        "http" | "https" => {
            let response = client.get(url).send().await?.error_for_status()?;
            transfer.start(response.content_length());
            let mut chunks = response.bytes_stream();
            while let Some(chunk) = chunks.next().await {
                let chunk = chunk?;
                file.write_all(&chunk)?;
                transfer.advance(chunk.len() as u64);
            }
        }
        scheme => bail!("unsupported URL scheme {scheme}"),
    }
    let mask = crate::source::startup_creation_mask().unwrap_or(0o022);
    file.as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o666 & !mask))?;
    file.as_file_mut().sync_all()?;
    file.persist(&request.destination)?;
    Ok(())
}
