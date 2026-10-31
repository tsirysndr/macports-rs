use super::{Options, Request};
use crate::progress::{Progress, TransferProgress};
use anyhow::{Context, Result, bail, ensure};
use futures_util::StreamExt;
use reqwest::{Client, Url};
use std::{io::Write, os::unix::fs::PermissionsExt, path::Path, time::Duration};

/// Fetch missing distfiles sequentially, in candidate order, like upstream.
pub async fn run(
    requests: &[Request],
    options: &Options,
    progress: &Progress,
    proxies: Option<&crate::install::proxy::Resolved>,
) -> Result<usize> {
    let agent = options
        .user_agent
        .clone()
        .unwrap_or_else(|| concat!("port-rs/", env!("CARGO_PKG_VERSION")).into());
    let builder = Client::builder()
        .user_agent(agent)
        .danger_accept_invalid_certs(options.ignore_sslcert)
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
        fetch(&client, request, options, progress).await?;
        fetched += 1;
    }
    Ok(fetched)
}

async fn fetch(
    client: &Client,
    request: &Request,
    options: &Options,
    progress: &Progress,
) -> Result<()> {
    let directory = request
        .destination
        .parent()
        .context("distfile destination has no directory")?;
    let transfer = progress.file(&request.filename);
    let mut errors = Vec::new();
    for (index, url) in request.urls.iter().enumerate() {
        if !options.prefix.is_empty() {
            let site = request.sites.get(index).unwrap_or(url);
            progress.phase_with_prefix(
                &options.prefix,
                &format!("Attempting to fetch {} from {site}", request.filename),
            );
        }
        match attempt(client, url, directory, request, options, &transfer).await {
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
    options: &Options,
    transfer: &TransferProgress,
) -> Result<()> {
    let url = Url::parse(url)?;
    let mut file = tempfile::Builder::new()
        .prefix(&format!("{}.TMP", request.filename))
        .tempfile_in(directory)?;
    let mut modified = None;
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
            let mut get = client.get(url);
            if let Some((user, password)) = &options.credentials {
                get = get.basic_auth(user, Some(password));
            }
            let response = get.send().await?.error_for_status()?;
            if options.remote_time {
                modified = response
                    .headers()
                    .get(reqwest::header::LAST_MODIFIED)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| httpdate::parse_http_date(value).ok());
            }
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
    if let Some(time) = modified {
        file.as_file().set_modified(time)?;
    }
    file.as_file_mut().sync_all()?;
    file.persist(&request.destination)?;
    Ok(())
}
