//! Binary image installation after the caller has resolved metadata and run the
//! required Portfile lifecycle. This backend does not execute hooks or source builds.
mod batch;
use super::{
    Action, Item, activation::Plan, archive::VerifiedArchive, extract, inspect::Limits,
    store::Store, target::Target,
};
use crate::{
    progress::Progress,
    registry::write::{NewImage, Writer},
};
use anyhow::{Context, Result, ensure};
pub use batch::{Batch, Job, fetch_batch, fetch_batch_with_proxies};
use std::path::{Path, PathBuf};

pub struct Installer<'a> {
    pub store: &'a Store,
    pub registry: &'a mut Writer,
    pub staging: &'a Path,
    pub root: &'a Path,
    pub limits: Limits,
    pub progress: &'a Progress,
}
pub struct InstalledImage {
    pub id: i64,
    pub location: PathBuf,
}
impl Installer<'_> {
    /// Install one authenticated archive. The caller must resolve provider
    /// checks/dependencies and exclusively manage the prefix and image store.
    /// File inventory and requested status come from the prepared image/plan;
    /// other registration metadata must come from evaluated build properties.
    /// If activation fails after registration, the inactive image is retained
    /// for retry, while activation rolls back its new payload paths.
    #[cfg(target_os = "macos")]
    pub fn install(
        &mut self,
        archive: &mut VerifiedArchive,
        item: &Item<'_>,
        target: &Target,
        mut registration: NewImage,
    ) -> Result<InstalledImage> {
        validate(item)?;
        self.progress.phase(&format!(
            "Preparing {} @{}{}",
            item.port.name,
            item.port.version(),
            item.variants
        ));
        let prepared =
            extract::prepare_for_target(archive, item, target, self.staging, self.limits)?;
        registration.files = prepared.registry_files()?;
        registration.requested = item.requested;
        self.progress.phase(&format!("Storing {}", item.port.name));
        let stored = self
            .store
            .stage_prepared(archive, item, target, &prepared)?;
        let location = stored.location().to_owned();
        self.progress
            .phase(&format!("Registering {}", item.port.name));
        let id = stored.register(self.registry, registration)?;
        self.progress
            .phase(&format!("Activating {}", item.port.name));
        let activation = (|| {
            let plan = Plan::inspect(&prepared, self.root)?;
            plan.activate(self.registry, id)
        })();
        activation.with_context(|| {
            format!(
                "could not activate registered image {} (id {id}, archive {}); retained for retry",
                item.port.name,
                location.display()
            )
        })?;
        self.progress.phase(&format!(
            "Installed {} @{}{}",
            item.port.name,
            item.port.version(),
            item.variants
        ));
        Ok(InstalledImage { id, location })
    }
}
fn validate(item: &Item<'_>) -> Result<()> {
    ensure!(
        item.action == Action::Install,
        "binary installation requires an install plan step"
    );
    ensure!(
        item.variants_resolved && item.provider_fallbacks.is_empty(),
        "binary installation requires resolved variants and dependency providers"
    );
    Ok(())
}
