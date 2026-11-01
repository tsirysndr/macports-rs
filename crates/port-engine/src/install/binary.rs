//! Binary image installation after the caller has resolved metadata and run the
//! required Portfile lifecycle, or with a prepared session via install_prepared.
//! This backend does not build source archives.
mod batch;
mod lifecycle;
mod selection;
use super::{
    Action, Item, activation::Plan, archive::VerifiedArchive, extract, inspect::Limits,
    store::Store, target::Target,
};
use crate::{
    progress::Progress,
    registry::write::{NewImage, Writer},
};
use anyhow::{Context, Result, ensure};
pub use batch::{
    Batch, Job, TrustedJob, fetch_batch, fetch_batch_with_proxies, fetch_trusted_batch,
};
pub use lifecycle::LifecycleResult;
pub use selection::select_inactive;
use std::path::{Path, PathBuf};

pub struct Installer<'a> {
    pub store: &'a Store,
    pub registry: &'a mut Writer,
    pub staging: &'a Path,
    pub root: &'a Path,
    pub limits: Limits,
    pub progress: &'a Progress,
    /// An image already prepared from the archive the next registration uses.
    pub prepared: Option<extract::PreparedImage>,
}
#[derive(Debug)]
pub struct InstalledImage {
    pub id: i64,
    pub location: PathBuf,
}
impl<'resources> Installer<'resources> {
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
        registration: NewImage,
    ) -> Result<InstalledImage> {
        let registered = self.register(archive, item, target, registration)?;
        registered.activate()
    }

    /// Store and register an inactive image, keeping its private prepared payload
    /// alive for activation after install hooks. Dropping the returned handle
    /// retains the registered archive for retry and discards temporary extraction.
    #[cfg(target_os = "macos")]
    pub fn register<'installer>(
        &'installer mut self,
        archive: &mut VerifiedArchive,
        item: &Item<'_>,
        target: &Target,
        mut registration: NewImage,
    ) -> Result<RegisteredImage<'installer, 'resources>> {
        validate(item)?;
        self.progress.phase(&format!(
            "Preparing {} @{}{}",
            item.port.name,
            item.port.version(),
            item.variants
        ));
        let prepared = match self.prepared.take() {
            Some(prepared) => extract::reuse_for_target(prepared, archive, item, target)?,
            None => extract::prepare_for_target(archive, item, target, self.staging, self.limits)?,
        };
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
        Ok(RegisteredImage {
            installer: self,
            id,
            location,
            prepared,
            name: item.port.name.clone(),
            label: format!(
                "{} @{}{}",
                item.port.name,
                item.port.version(),
                item.variants
            ),
        })
    }
}

/// Inactive registered image bound to the installer that owns its registry.
/// This handle separates install and activate hook boundaries without publishing
/// files early or allowing its registry ID to be replaced by the caller.
pub struct RegisteredImage<'installer, 'resources> {
    installer: &'installer mut Installer<'resources>,
    id: i64,
    location: PathBuf,
    prepared: extract::PreparedImage,
    name: String,
    label: String,
}
impl RegisteredImage<'_, '_> {
    pub fn id(&self) -> i64 {
        self.id
    }
    pub fn location(&self) -> &Path {
        &self.location
    }

    #[cfg(target_os = "macos")]
    pub fn activate(self) -> Result<InstalledImage> {
        self.activate_inner(true)
    }

    #[cfg(target_os = "macos")]
    fn activate_inner(self, announce_completion: bool) -> Result<InstalledImage> {
        let Self {
            installer,
            id,
            location,
            prepared,
            name,
            label,
        } = self;
        installer.progress.phase(&format!("Activating {name}"));
        let activation = (|| {
            let plan = Plan::inspect(&prepared, installer.root)?;
            plan.activate(installer.registry, id)
        })();
        activation.with_context(|| format!(
            "could not activate registered image {name} (id {id}, archive {}); retained for retry",
            location.display()
        ))?;
        if announce_completion {
            installer.progress.phase(&format!("Installed {label}"));
        }
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
