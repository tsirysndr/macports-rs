//! `port archivefetch` (portarchivefetch) and the binary-first path of
//! `install`: fetch a port's signed binary archive from compatible archive
//! sites and verify it with the sites' trusted keys.
use super::local::Host;
use crate::{
    index::Index,
    install::{self, Item, archive::VerifiedArchive, fetch, target::Target},
    portfile::Session,
    query,
    source::Platform,
};
use anyhow::{Context, Result, ensure};
use std::path::Path;

const MAX_ARCHIVE_BYTES: u64 = 16 << 30;

/// Download one URL into `path`.
fn download(url: &str, path: &Path) -> Result<()> {
    let bytes = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let response = reqwest::get(url).await?;
            ensure!(
                response.status().is_success(),
                "{url}: HTTP {}",
                response.status().as_u16()
            );
            anyhow::Ok(response.bytes().await?)
        })?;
    std::fs::write(path, &bytes)?;
    Ok(())
}

/// Archives from RSA-signed sites: the first whose `.rmd160` verifies.
fn fetch_legacy(
    host: &Host,
    session: &mut Session,
    item: &Item<'_>,
    machine: &Platform,
    staging: &Path,
) -> Result<VerifiedArchive> {
    let selection = session
        .archive_selection()?
        .context("no archive sites are configured")?;
    let target = Target::from_session(session, machine)?;
    let request = selection.legacy_request(session, &target, item)?;
    let config = host
        .config
        .get("pubkeys_conf")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/opt/local/etc/macports/pubkeys.conf".into());
    let mut keys = request.keys;
    keys.extend(install::sites::legacy::configured_pem_keys(
        &config,
        &std::env::current_dir()?,
    ));
    ensure!(
        !keys.is_empty(),
        "no RSA keys are configured for {}",
        item.port.name
    );
    let filename = target.filename(item)?;
    let archive = staging.join(&filename);
    let signature = staging.join(format!("{filename}.rmd160"));
    host.progress
        .phase(&format!("Fetching archive for {}", item.port.name));
    for url in &request.urls {
        if download(url, &archive).is_err()
            || download(&format!("{url}.rmd160"), &signature).is_err()
        {
            continue;
        }
        if install::sites::legacy::verify(&archive, &signature, &keys) {
            return VerifiedArchive::local(std::fs::File::open(&archive)?);
        }
        host.progress
            .warning(&format!("Failed to verify signature for archive {url}"));
    }
    anyhow::bail!(
        "no verified archive for {} from RSA-signed sites",
        item.port.name
    )
}

/// Fetch and authenticate the archive for a planned item into `staging`,
/// from Signify-signed sites or, failing that, RSA-signed ones.
pub(super) fn fetch_archive(
    host: &Host,
    session: &mut Session,
    item: &Item<'_>,
    machine: &Platform,
    staging: &Path,
) -> Result<VerifiedArchive> {
    match fetch_signify(host, session, item, machine, staging) {
        Ok(archive) => Ok(archive),
        Err(signify) => fetch_legacy(host, session, item, machine, staging)
            .map_err(|legacy| signify.context(format!("{legacy:#}"))),
    }
}

fn fetch_signify(
    host: &Host,
    session: &mut Session,
    item: &Item<'_>,
    machine: &Platform,
    staging: &Path,
) -> Result<VerifiedArchive> {
    let requests = [signify_request(host, session, item, machine)?];
    host.progress
        .phase(&format!("Fetching archive for {}", item.port.name));
    let mut archives = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(fetch::run_trusted(
            &requests,
            staging,
            1,
            &host.progress,
            &host.context.proxies,
        ))?;
    archives.pop().context("archive fetch returned nothing")
}

/// The Signify-signed archive request for an item, with its sites' keys.
fn signify_request(
    host: &Host,
    session: &mut Session,
    item: &Item<'_>,
    machine: &Platform,
) -> Result<fetch::TrustedRequest> {
    let selection = session
        .archive_selection()?
        .context("no archive sites are configured")?;
    let target = Target::from_session(session, machine)?;
    let prepared = selection.request_from_session(session, &target, item, MAX_ARCHIVE_BYTES)?;
    let trust =
        host.paths
            .archive_site_keys(&host.config, &selection, &prepared.references, "tbz2")?;
    ensure!(
        !trust.keys.is_empty(),
        "no trusted keys for {}",
        item.port.name
    );
    Ok(fetch::TrustedRequest {
        request: prepared.request,
        keys: trust.keys,
    })
}

/// Download the archives of several items at once (`fetch_threads`
/// concurrent transfers). Items without a Signify request or whose download
/// fails are left out, to be fetched or built one by one later.
pub(super) fn prefetch(
    host: &Host,
    sessions: &mut std::collections::BTreeMap<String, Session>,
    items: &[&Item<'_>],
    machine: &Platform,
    staging: &Path,
) -> std::collections::HashMap<String, VerifiedArchive> {
    let mut names = Vec::new();
    let mut requests = Vec::new();
    for item in items {
        let Some(session) = sessions.get_mut(&item.port.name) else {
            continue;
        };
        match signify_request(host, session, item, machine) {
            Ok(request) => {
                names.push(item.port.name.clone());
                requests.push(request);
            }
            Err(err) => host.progress.log(
                "debug",
                &format!("no signed archive request for {}: {err:#}", item.port.name),
            ),
        }
    }
    if requests.is_empty() {
        return Default::default();
    }
    for name in &names {
        host.progress.phase(&format!("Fetching archive for {name}"));
    }
    let jobs = host
        .context
        .variables
        .get("fetch_threads")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(4)
        .clamp(1, 128)
        .min(requests.len());
    let results = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(anyhow::Error::from)
        .and_then(|runtime| {
            runtime.block_on(fetch::run_trusted_each(
                &requests,
                staging,
                jobs,
                &host.progress,
                &host.context.proxies,
            ))
        });
    let results = match results {
        Ok(results) => results,
        Err(err) => {
            host.progress
                .log("debug", &format!("archive prefetch failed: {err:#}"));
            return Default::default();
        }
    };
    names
        .into_iter()
        .zip(results)
        .filter_map(|(name, result)| match result {
            Ok(archive) => Some((name, archive)),
            Err(err) => {
                host.progress
                    .log("debug", &format!("no archive for {name}: {err:#}"));
                None
            }
        })
        .collect()
}

/// Whether a binary archive exists for the evaluated port (upstream
/// `_archive_available`): a verified local archive, or one an archive site
/// answers for.
pub(super) fn available(host: &Host, session: &mut Session, machine: &Platform) -> bool {
    if super::archive_mode() == super::ArchiveMode::SourceOnly {
        return false;
    }
    let evaluation = session.metadata().clone();
    let item = Item {
        port: &evaluation.port,
        variants: evaluation.enabled.clone(),
        variants_resolved: true,
        provider_fallbacks: Vec::new(),
        requested: false,
        action: install::Action::Install,
    };
    let urls = (|| -> Result<Vec<String>> {
        let target = Target::from_session(session, machine)?;
        let filename = target.filename(&item)?;
        if let Some(portdbpath) = host.context.variables.get("portdbpath")
            && Path::new(portdbpath)
                .join("incoming/verified")
                .join(&filename)
                .is_file()
        {
            return Ok(Vec::new());
        }
        let selection = session
            .archive_selection()?
            .context("no archive sites are configured")?;
        let mut urls = selection
            .request_from_session(session, &target, &item, MAX_ARCHIVE_BYTES)
            .map(|prepared| prepared.request.urls)
            .unwrap_or_default();
        if let Ok(legacy) = selection.legacy_request(session, &target, &item) {
            urls.extend(legacy.urls);
        }
        ensure!(!urls.is_empty(), "no archive URLs");
        Ok(urls)
    })();
    let urls = match urls {
        Ok(urls) if urls.is_empty() => return true,
        Ok(urls) => urls,
        Err(err) => {
            host.progress.log(
                "debug",
                &format!("no archive for {}: {err:#}", evaluation.port.name),
            );
            return false;
        }
    };
    let found = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()
        .and_then(|runtime| {
            runtime.block_on(async {
                let client = reqwest::Client::builder()
                    .connect_timeout(std::time::Duration::from_secs(10))
                    .timeout(std::time::Duration::from_secs(30))
                    .build()
                    .ok()?;
                for url in &urls {
                    if let Ok(response) = client.head(url).send().await
                        && response.status().is_success()
                    {
                        return Some(url.clone());
                    }
                }
                None
            })
        });
    host.progress.log(
        "debug",
        &match &found {
            Some(url) => format!("archive for {} is available at {url}", evaluation.port.name),
            None => format!("no archive found for {}", evaluation.port.name),
        },
    );
    found.is_some()
}

/// `port archivefetch`: fetch and verify archives into `incoming/verified`.
pub(super) fn run(index: &Index, ports: &[String], quiet: bool, root: &Path) -> Result<()> {
    let host = Host::new(root, None, quiet)?;
    let machine = crate::source::detect_platform()?;
    let portdbpath = host
        .context
        .variables
        .get("portdbpath")
        .cloned()
        .context("portdbpath is not configured")?;
    let verified = Path::new(&portdbpath).join("incoming/verified");
    std::fs::create_dir_all(&verified)?;
    let install::SessionPlan { plan, mut sessions } = plan_requested(&host, index, ports, root)?;
    for item in plan.items().filter(|item| item.requested) {
        let session = sessions
            .get_mut(&item.port.name)
            .with_context(|| format!("{}: no evaluated session", item.port.name))?;
        let staging = tempfile::tempdir_in(&verified)?;
        let mut archive = fetch_archive(&host, session, &item, &machine, staging.path())?;
        let filename = Target::from_session(session, &machine)?.filename(&item)?;
        let destination = verified.join(&filename);
        let mut file = tempfile::NamedTempFile::new_in(&verified)?;
        std::io::copy(&mut archive.reader()?, &mut file)?;
        file.persist(&destination)?;
        std::fs::set_permissions(
            &destination,
            std::os::unix::fs::PermissionsExt::from_mode(0o644),
        )?;
        std::fs::write(
            format!("{}.sig", destination.display()),
            archive.signature(),
        )?;
        host.progress.phase(&format!("Archive {filename} verified"));
    }
    Ok(())
}

/// The archive for an item: a verified download, the installed image, or a
/// fresh archivefetch into `staging`. `None` when there is none.
fn find_archive(
    host: &Host,
    session: &mut Session,
    item: &Item<'_>,
    machine: &Platform,
    staging: &Path,
) -> Result<Option<std::path::PathBuf>> {
    if super::archive_mode() == super::ArchiveMode::SourceOnly {
        return Ok(None);
    }
    let portdbpath = host
        .context
        .variables
        .get("portdbpath")
        .cloned()
        .context("portdbpath is not configured")?;
    let filename = Target::from_session(session, machine)?.filename(item)?;
    let candidate = Path::new(&portdbpath)
        .join("incoming/verified")
        .join(&filename);
    if candidate.is_file() {
        return Ok(Some(candidate));
    }
    if let Some(registry) = crate::registry::Registry::open(Some(&host.registry_path))
        .ok()
        .flatten()
    {
        for entry in registry.entries()? {
            if entry.name.eq_ignore_ascii_case(&item.port.name)
                && entry.version == item.port.get("version")
                && entry.variants == item.variants
                && let Ok(location) = registry.location(&entry)
                && Path::new(&location).is_file()
            {
                return Ok(Some(location.into()));
            }
        }
    }
    match fetch_archive(host, session, item, machine, staging) {
        Ok(mut fetched) => {
            let path = staging.join(&filename);
            std::io::copy(&mut fetched.reader()?, &mut std::fs::File::create(&path)?)?;
            Ok(Some(path))
        }
        Err(err) => {
            host.progress.log(
                "debug",
                &format!("no archive for {}: {err:#}", item.port.name),
            );
            Ok(None)
        }
    }
}

/// Unpack the item's archive into its destroot (upstream unarchive). Returns
/// false when there is no archive and the destroot must be built instead.
pub(super) fn unpack(
    host: &Host,
    session: &mut Session,
    item: &Item<'_>,
    machine: &Platform,
) -> Result<bool> {
    let staging = tempfile::tempdir()?;
    let label = format!(
        "{} {}_{}{}",
        session.scalar("subport")?,
        session.scalar("version")?,
        session.scalar("revision")?,
        item.variants
    );
    let Some(archive) = find_archive(host, session, item, machine, staging.path())? else {
        ensure!(
            super::archive_mode() != super::ArchiveMode::BinaryOnly,
            "Archive for {label} not found, required when binary-only is set!"
        );
        return Ok(false);
    };
    let extension = archive
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned())
        .unwrap_or_default();
    host.progress
        .phase(&format!("Unpacking {extension} archive for {label}"));
    let destroot = std::path::PathBuf::from(session.scalar("destroot")?);
    std::fs::create_dir_all(&destroot)?;
    let status = std::process::Command::new("/usr/bin/tar")
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(&destroot)
        .status()
        .context("cannot run tar")?;
    ensure!(status.success(), "unpacking {} failed", archive.display());
    for entry in std::fs::read_dir(&destroot)?.flatten() {
        if entry.file_name().to_string_lossy().starts_with('+') && entry.path().is_file() {
            std::fs::remove_file(entry.path())?;
        }
    }
    host.progress.log(
        "info",
        &format!(
            "Archive {} unpacked",
            archive.file_name().unwrap_or_default().to_string_lossy()
        ),
    );
    Ok(true)
}

/// `port unarchive`: unpack each port's archive into its destroot.
pub(super) fn unarchive(index: &Index, ports: &[String], quiet: bool, root: &Path) -> Result<()> {
    let host = Host::new(root, None, quiet)?;
    let machine = crate::source::detect_platform()?;
    let worker =
        crate::install::mirror_worker::MirrorWorker::new(host.context.mirror_timings.clone(), 8)?;
    let install::SessionPlan { plan, mut sessions } = plan_requested(&host, index, ports, root)?;
    for item in plan.items().filter(|item| item.requested) {
        let name = item.port.name.clone();
        let session = sessions
            .get_mut(&name)
            .with_context(|| format!("{name}: no evaluated session"))?;
        super::phase::prepare(session, &host, &worker, &name)?;
        host.progress.close_log();
        if !unpack(&host, session, &item, &machine)? {
            host.progress.log(
                "debug",
                &format!("Skipping unarchive ({name}) since no suitable archive found"),
            );
        }
    }
    Ok(())
}

/// Plan the requested ports (with configured variants) and their sessions.
pub(super) fn plan_requested(
    host: &Host,
    index: &Index,
    ports: &[String],
    root: &Path,
) -> Result<install::SessionPlan> {
    let mut specs = query::specifications(index, ports)?;
    for spec in &mut specs {
        host.context.variants.apply(spec);
    }
    let registry = host.context.open_registry(None)?;
    install::plan_source_sessions(index, &specs, registry.as_ref(), |port, spec| {
        host.open(root, port, spec)
    })
}

/// Unpack and inspect downloaded archives on every core before installing in
/// order; a failure leaves that port to the sequential path, which reports it.
pub(super) fn prepare_all(
    host: &Host,
    sessions: &mut std::collections::BTreeMap<String, Session>,
    items: &[Item<'_>],
    archives: &mut std::collections::HashMap<String, VerifiedArchive>,
    machine: &Platform,
    staging: &Path,
) -> std::collections::HashMap<String, install::extract::PreparedImage> {
    let mut jobs = Vec::new();
    for item in items {
        let (Some(session), Some(archive)) = (
            sessions.get_mut(&item.port.name),
            archives.remove(&item.port.name),
        ) else {
            continue;
        };
        match Target::from_session(session, machine) {
            Ok(target) => jobs.push((item, target, archive)),
            Err(_) => {
                archives.insert(item.port.name.clone(), archive);
            }
        }
    }
    if jobs.len() < 2 {
        for (item, _, archive) in jobs {
            archives.insert(item.port.name.clone(), archive);
        }
        return Default::default();
    }
    let threads = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(jobs.len());
    host.progress.phase(&format!(
        "Preparing {} archives ({threads} concurrent)",
        jobs.len()
    ));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let slots: Vec<_> = jobs.into_iter().map(std::sync::Mutex::new).collect();
    let prepared: Vec<_> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(slot) = slots.get(index) else {
                            return done;
                        };
                        let mut job = slot.lock().unwrap();
                        let (item, target, archive) = &mut *job;
                        let image = install::extract::prepare_for_target(
                            archive,
                            item,
                            target,
                            staging,
                            install::inspect::Limits::default(),
                        );
                        done.push((item.port.name.clone(), image));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().expect("archive preparation panicked"))
            .collect()
    });
    for slot in slots {
        let (item, _, archive) = slot.into_inner().unwrap();
        archives.insert(item.port.name.clone(), archive);
    }
    prepared
        .into_iter()
        .filter_map(|(name, image)| match image {
            Ok(image) => Some((name, image)),
            Err(err) => {
                host.progress
                    .log("debug", &format!("cannot prepare {name} ahead: {err:#}"));
                None
            }
        })
        .collect()
}
