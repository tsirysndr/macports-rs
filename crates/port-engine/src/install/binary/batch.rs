use super::{InstalledImage, Installer, Item, NewImage, Progress, Target, VerifiedArchive};
use crate::install::{
    fetch::{self, Request},
    signature::TrustedKey,
};
use anyhow::{Result, ensure};
use std::{collections::HashMap, path::Path};

/// One resolved installation step, in dependency-first order.
pub struct Job<'a> {
    pub item: Item<'a>,
    pub target: Target,
    pub request: Request,
    pub registration: NewImage,
}

/// One installation step with the trust keys selected for that port.
pub struct TrustedJob<'a> {
    pub job: Job<'a>,
    pub keys: Vec<TrustedKey>,
}

/// Authenticate every job with its own trust set before allowing any installation.
pub async fn fetch_trusted_batch<'a>(
    jobs: Vec<TrustedJob<'a>>,
    staging: &Path,
    concurrency: usize,
    progress: &Progress,
    proxies: &crate::install::proxy::Resolved,
) -> Result<Batch<'a>> {
    let (jobs, keys): (Vec<_>, Vec<_>) = jobs
        .into_iter()
        .map(|entry| (entry.job, entry.keys))
        .unzip();
    validate_jobs(&jobs)?;
    let (requests, steps): (Vec<_>, Vec<_>) = jobs
        .into_iter()
        .zip(keys)
        .map(|(job, keys)| {
            (
                fetch::TrustedRequest {
                    request: job.request,
                    keys,
                },
                (job.item, job.target, job.registration),
            )
        })
        .unzip();
    let archives = fetch::run_trusted(&requests, staging, concurrency, progress, proxies).await?;
    Ok(Batch { steps, archives })
}

/// All downloads authenticated, with original plan order retained. No registry
/// or prefix changes occur until `install` is called. Dropping discards staging.
pub struct Batch<'a> {
    steps: Vec<(Item<'a>, Target, NewImage)>,
    archives: Vec<VerifiedArchive>,
}
pub async fn fetch_batch<'a>(
    jobs: Vec<Job<'a>>,
    staging: &Path,
    keys: &[TrustedKey],
    concurrency: usize,
    progress: &Progress,
) -> Result<Batch<'a>> {
    fetch_batch_inner(jobs, staging, keys, concurrency, progress, None).await
}

/// Fetch a binary batch using the startup driver's resolved proxy snapshot.
pub async fn fetch_batch_with_proxies<'a>(
    jobs: Vec<Job<'a>>,
    staging: &Path,
    keys: &[TrustedKey],
    concurrency: usize,
    progress: &Progress,
    proxies: &crate::install::proxy::Resolved,
) -> Result<Batch<'a>> {
    fetch_batch_inner(jobs, staging, keys, concurrency, progress, Some(proxies)).await
}

async fn fetch_batch_inner<'a>(
    jobs: Vec<Job<'a>>,
    staging: &Path,
    keys: &[TrustedKey],
    concurrency: usize,
    progress: &Progress,
    proxies: Option<&crate::install::proxy::Resolved>,
) -> Result<Batch<'a>> {
    validate_jobs(&jobs)?;
    let (requests, steps): (Vec<_>, Vec<_>) = jobs
        .into_iter()
        .map(|job| (job.request, (job.item, job.target, job.registration)))
        .unzip();
    let archives = match proxies {
        Some(proxies) => {
            fetch::run_with_proxies(&requests, staging, keys, concurrency, progress, proxies)
                .await?
        }
        None => fetch::run(&requests, staging, keys, concurrency, progress).await?,
    };
    Ok(Batch { steps, archives })
}
impl Batch<'_> {
    /// Install authenticated downloads through retained Portfile sessions in
    /// dependency order. Sessions must match the entire batch before any hooks
    /// execute. Resolve registration/providers freshly after each pre-install;
    /// downloaded target coordinates cannot change underneath the batch.
    #[cfg(target_os = "macos")]
    pub fn install_prepared(
        self,
        installer: &mut Installer<'_>,
        sessions: &mut [crate::portfile::Session],
        mut registration: impl FnMut(
            &mut crate::portfile::Session,
            &Item<'_>,
        ) -> Result<(Target, NewImage)>,
    ) -> Result<Vec<super::LifecycleResult>> {
        ensure!(
            sessions.len() == self.steps.len(),
            "binary batch session count does not match the plan"
        );
        for (session, (item, _, _)) in sessions.iter_mut().zip(&self.steps) {
            ensure!(
                session
                    .scalar("subport")?
                    .eq_ignore_ascii_case(&item.port.name),
                "binary batch session order does not match the plan for {}",
                item.port.name
            );
        }
        let positions: HashMap<_, _> = self
            .steps
            .iter()
            .enumerate()
            .map(|(i, (item, _, _))| (item.port.name.to_ascii_lowercase(), i))
            .collect();
        self.steps
            .into_iter()
            .zip(self.archives)
            .zip(sessions.iter_mut())
            .enumerate()
            .map(|(index, (((item, target, _), mut archive), session))| {
                installer.install_prepared(&mut archive, &item, session, false, |session| {
                    let (live_target, record) = registration(session, &item)?;
                    ensure!(
                        live_target == target,
                        "archive target changed after download for {}; replan",
                        item.port.name
                    );
                    for dependency in &record.dependencies {
                        ensure!(
                            positions
                                .get(&dependency.to_ascii_lowercase())
                                .is_none_or(|position| *position < index),
                            "live binary batch is not dependency-first: {} requires {dependency}",
                            item.port.name
                        );
                    }
                    Ok((live_target, record))
                })
            })
            .collect()
    }

    /// Run blocking filesystem/registry work in plan order after asynchronous
    /// fetching. Stop on the first failure; earlier installed dependencies remain
    /// installed, and unprocessed anonymous archives are discarded.
    #[cfg(target_os = "macos")]
    pub fn install(self, installer: &mut Installer<'_>) -> Result<Vec<InstalledImage>> {
        self.steps
            .into_iter()
            .zip(self.archives)
            .map(|((item, target, registration), mut archive)| {
                installer.install(&mut archive, &item, &target, registration)
            })
            .collect()
    }
}

fn validate_jobs(jobs: &[Job<'_>]) -> Result<()> {
    let mut positions = HashMap::new();
    for (index, job) in jobs.iter().enumerate() {
        super::validate(&job.item)?;
        job.target.filename(&job.item)?;
        ensure!(
            positions
                .insert(job.item.port.name.to_ascii_lowercase(), index)
                .is_none(),
            "duplicate port in binary installation batch"
        );
    }
    for (index, job) in jobs.iter().enumerate() {
        for dependency in &job.registration.dependencies {
            ensure!(
                positions
                    .get(&dependency.to_ascii_lowercase())
                    .is_none_or(|position| *position < index),
                "binary installation batch is not dependency-first: {} requires {dependency}",
                job.item.port.name
            );
        }
    }
    Ok(())
}
