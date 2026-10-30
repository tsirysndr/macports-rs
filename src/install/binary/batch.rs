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
    let (requests, steps): (Vec<_>, Vec<_>) = jobs
        .into_iter()
        .map(|job| (job.request, (job.item, job.target, job.registration)))
        .unzip();
    let archives = fetch::run(&requests, staging, keys, concurrency, progress).await?;
    Ok(Batch { steps, archives })
}
impl Batch<'_> {
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
