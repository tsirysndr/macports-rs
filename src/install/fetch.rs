//! Concurrent signed-archive transport. URLs are candidates, never trust anchors.
use super::{archive::VerifiedArchive, signature::TrustedKey};
use crate::progress::{Progress, TransferProgress};
use anyhow::{Context, Result, bail, ensure};
use futures_util::{StreamExt, stream};
use reqwest::{Client, StatusCode, Url};
use std::{path::Path, time::Duration};
use tokio::io::AsyncWriteExt;

pub struct Request {
    pub name: String,
    /// Ordered archive mirrors. The corresponding signature URL appends `.sig`.
    pub urls: Vec<String>,
    pub max_bytes: u64,
}

/// Fetch in input order with bounded concurrency. Any failure drops all staged
/// results, after draining the remaining transfers. Nothing is published.
pub async fn run(
    requests: &[Request],
    directory: &Path,
    keys: &[TrustedKey],
    jobs: usize,
    progress: &Progress,
) -> Result<Vec<VerifiedArchive>> {
    run_inner(requests, directory, keys, jobs, progress, None).await
}

/// Fetch using explicit startup proxy settings instead of process environment.
pub async fn run_with_proxies(
    requests: &[Request],
    directory: &Path,
    keys: &[TrustedKey],
    jobs: usize,
    progress: &Progress,
    proxies: &super::proxy::Resolved,
) -> Result<Vec<VerifiedArchive>> {
    run_inner(requests, directory, keys, jobs, progress, Some(proxies)).await
}

async fn run_inner(
    requests: &[Request],
    directory: &Path,
    keys: &[TrustedKey],
    jobs: usize,
    progress: &Progress,
    proxies: Option<&super::proxy::Resolved>,
) -> Result<Vec<VerifiedArchive>> {
    ensure!((1..=128).contains(&jobs), "jobs must be between 1 and 128");
    ensure!(
        !keys.is_empty() && keys.len() <= 64,
        "invalid trusted key count"
    );
    // Validate the entire batch before any network or filesystem work.
    let urls: Vec<Vec<Url>> = requests
        .iter()
        .map(|request| {
            ensure!(request.max_bytes > 0, "archive size limit must be positive");
            ensure!(
                !request.urls.is_empty(),
                "no archive mirrors for {}",
                request.name
            );
            request
                .urls
                .iter()
                .map(|value| {
                    let url = Url::parse(value)?;
                    ensure!(
                        matches!(url.scheme(), "https" | "http") && url.host_str().is_some(),
                        "archive URLs require HTTP(S)"
                    );
                    ensure!(
                        url.username().is_empty()
                            && url.password().is_none()
                            && url.query().is_none()
                            && url.fragment().is_none(),
                        "archive URLs cannot contain credentials, queries, or fragments"
                    );
                    Ok(url)
                })
                .collect()
        })
        .collect::<Result<_>>()?;
    let builder = Client::builder()
        .user_agent(concat!("port-rs/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .timeout(Duration::from_secs(3600));
    let client = match proxies {
        Some(proxies) => proxies.configure_client(builder)?,
        None => builder,
    }
    .build()?;
    progress.phase(&format!(
        "Fetching {} signed archives ({jobs} concurrent transfers)",
        requests.len()
    ));
    let mut results: Vec<(usize, Result<VerifiedArchive>)> = stream::iter(
        requests
            .iter()
            .zip(&urls)
            .enumerate()
            .map(|(index, (request, urls))| {
                let future = fetch(&client, request, urls, directory, keys, progress);
                async move { (index, future.await) }
            }),
    )
    .buffer_unordered(jobs)
    .collect()
    .await;
    // Free each finished slot immediately, even if an earlier transfer is slow.
    // Restore plan order only after all transfers finish, including on failure.
    results.sort_unstable_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, result)| result).collect()
}

async fn fetch(
    client: &Client,
    request: &Request,
    urls: &[Url],
    directory: &Path,
    keys: &[TrustedKey],
    progress: &Progress,
) -> Result<VerifiedArchive> {
    let transfer = progress.file(&request.name);
    let mut errors = Vec::new();
    for url in urls {
        match attempt(client, url, directory, keys, request.max_bytes, &transfer).await {
            Ok(archive) => {
                transfer.verified();
                return Ok(archive);
            }
            Err(error) => errors.push(format!("{url}: {error:#}")),
        }
    }
    bail!(
        "cannot fetch authenticated archive for {}: {}",
        request.name,
        errors.join("; ")
    )
}

async fn response(client: &Client, url: Url, limit: u64) -> Result<reqwest::Response> {
    let response = client.get(url).send().await?.error_for_status()?;
    ensure!(
        response.status() == StatusCode::OK,
        "expected a complete HTTP 200 response"
    );
    ensure!(
        response.content_length().is_none_or(|size| size <= limit),
        "download exceeds configured size limit"
    );
    Ok(response)
}

async fn attempt(
    client: &Client,
    url: &Url,
    directory: &Path,
    keys: &[TrustedKey],
    limit: u64,
    transfer: &TransferProgress,
) -> Result<VerifiedArchive> {
    let signature_url = Url::parse(&format!("{url}.sig"))?;
    let mut chunks = response(client, signature_url, 4096).await?.bytes_stream();
    let mut signature = Vec::new();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk?;
        ensure!(
            chunk.len() <= 4096 - signature.len(),
            "signature exceeds configured size limit"
        );
        signature.extend_from_slice(&chunk);
    }
    let signature = String::from_utf8(signature).context("signature is not UTF-8")?;
    let response = response(client, url.clone(), limit).await?;
    transfer.start(response.content_length());
    let directory = directory.to_owned();
    let file = tokio::task::spawn_blocking(move || tempfile::tempfile_in(directory)).await??;
    let mut file = tokio::fs::File::from_std(file);
    let mut remaining = limit;
    let mut chunks = response.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk?;
        ensure!(
            chunk.len() as u64 <= remaining,
            "archive exceeds configured size limit"
        );
        file.write_all(&chunk).await?;
        remaining -= chunk.len() as u64;
        transfer.advance(chunk.len() as u64);
    }
    file.flush().await?;
    let file = file.into_std().await;
    let keys = keys.to_vec();
    transfer.message("verifying signature");
    tokio::task::spawn_blocking(move || VerifiedArchive::authenticate(file, &signature, &keys))
        .await?
}
