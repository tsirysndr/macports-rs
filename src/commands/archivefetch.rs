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
    let mut specs = query::specifications(index, ports)?;
    for spec in &mut specs {
        host.context.variants.apply(spec);
    }
    let registry = host.context.open_registry(None)?;
    let plan = install::plan_source_sessions(index, &specs, registry.as_ref(), |port, spec| {
        host.open(root, port, spec)
    })?;
    drop(registry);
    let install::SessionPlan { plan, mut sessions } = plan;
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
