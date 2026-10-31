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

/// Fetch and authenticate the archive for a planned item into `staging`.
pub(super) fn fetch_archive(
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
    let prepared = selection.request_from_session(session, &target, item, MAX_ARCHIVE_BYTES)?;
    let trust =
        host.paths
            .archive_site_keys(&host.config, &selection, &prepared.references, "tbz2")?;
    ensure!(
        !trust.keys.is_empty(),
        "no trusted keys for {}",
        item.port.name
    );
    host.progress
        .phase(&format!("Fetching archive for {}", item.port.name));
    let requests = [fetch::TrustedRequest {
        request: prepared.request,
        keys: trust.keys,
    }];
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
