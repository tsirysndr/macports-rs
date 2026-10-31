//! Prepared install/activate orchestration for an authenticated binary image.
use super::*;
use crate::portfile::{Session, TargetOutcome};

#[derive(Debug)]
pub struct LifecycleResult {
    pub install: TargetOutcome,
    pub activate: TargetOutcome,
    /// Present only when the native activation body ran. Custom target bodies
    /// remain responsible for their own registry and filesystem effects.
    pub image: Option<InstalledImage>,
}
impl Installer<'_> {
    /// Activate an existing inactive registry image from a freshly verified
    /// archive (for example Store::load). No registration or store writes occur.
    /// Registry identity and file ownership are checked against the prepared
    /// archive before publishing. The host resolves live target/provider state
    /// in `target` after pre-activate hooks; dry runs/custom bodies skip it.
    #[cfg(target_os = "macos")]
    pub fn activate_prepared(
        &mut self,
        archive: &mut VerifiedArchive,
        item: &Item<'_>,
        image_id: i64,
        session: &mut Session,
        dry_run: bool,
        target: impl FnOnce(&mut Session) -> Result<Target>,
    ) -> Result<TargetOutcome> {
        ensure!(
            item.action == super::Action::Activate,
            "existing image activation requires an activate plan step"
        );
        ensure!(
            item.variants_resolved && item.provider_fallbacks.is_empty(),
            "existing image activation requires resolved variants and dependency providers"
        );
        session.set_progress(self.progress.clone());
        let activate_target = session.scalar("org.macports.activate")?;
        let mut activated = false;
        let outcome = session.run_prepared_target_with_native_body(
            &activate_target,
            dry_run,
            "portactivate::activate_main",
            |session| {
                let target = target(session)?;
                let prepared =
                    extract::prepare_for_target(archive, item, &target, self.staging, self.limits)?;
                self.progress
                    .phase(&format!("Activating {}", item.port.name));
                Plan::inspect(&prepared, self.root)?.activate(self.registry, image_id)?;
                activated = true;
                Ok(())
            },
        )?;
        if activated {
            self.progress.phase(&format!(
                "Activated {} @{}{}",
                item.port.name,
                item.port.version(),
                item.variants
            ));
        }
        Ok(outcome)
    }

    /// Run install and activate with live Portfile hooks and runtime cleanup.
    /// Dependencies, archivefetch and host execution context must already be
    /// prepared. Derive registration from the live session in `registration`
    /// (normally using registration::with_providers); it runs after pre-install.
    /// Dry runs and custom install bodies do not call the registration factory.
    /// An error after registration retains the inactive image; an error after
    /// activation retains the installed image. Neither claims hook completion.
    #[cfg(target_os = "macos")]
    pub fn install_prepared(
        &mut self,
        archive: &mut VerifiedArchive,
        item: &Item<'_>,
        session: &mut Session,
        dry_run: bool,
        registration: impl FnOnce(&mut Session) -> Result<(Target, NewImage)>,
    ) -> Result<LifecycleResult> {
        validate(item)?;
        session.set_progress(self.progress.clone());
        let install_target = session.scalar("org.macports.install")?;
        let mut pending = None;
        let progress = self.progress.clone();
        let install = session.run_prepared_target_with_native_body(
            &install_target,
            dry_run,
            "portinstall::install_main",
            |session| {
                let (target, record) = registration(session)?;
                pending = Some(self.register(archive, item, &target, record)?);
                Ok(())
            },
        )?;
        // Resolve activation only after install hooks: hooks may change its
        // metadata or replace the procedure, as in the interpreter driver.
        let activate_target = session.scalar("org.macports.activate")?;
        let mut image = None;
        let activate = session.run_prepared_target_with_native_body(
            &activate_target, dry_run, "portactivate::activate_main", |_| {
                let registered = pending.take().context(
                    "native activation needs a registered binary image; custom or skipped install requires registry image recovery"
                )?;
                image = Some(registered.activate_inner(false)?);
                Ok(())
            },
        )?;
        if image.is_some() {
            progress.phase(&format!(
                "Installed {} @{}{}",
                item.port.name,
                item.port.version(),
                item.variants
            ));
        }
        Ok(LifecycleResult {
            install,
            activate,
            image,
        })
    }

    /// Like `install_prepared`, for an image built from this session's destroot:
    /// the native install body resolves registration, then `package` creates
    /// the local image (as upstream's install_main archives the destroot).
    #[cfg(target_os = "macos")]
    pub fn install_built(
        &mut self,
        item: &Item<'_>,
        session: &mut Session,
        dry_run: bool,
        registration: impl FnOnce(&mut Session) -> Result<(Target, NewImage)>,
        package: impl FnOnce(&mut Session, &Target) -> Result<VerifiedArchive>,
    ) -> Result<LifecycleResult> {
        validate(item)?;
        session.set_progress(self.progress.clone());
        let install_target = session.scalar("org.macports.install")?;
        let mut pending = None;
        let progress = self.progress.clone();
        let install = session.run_prepared_target_with_native_body(
            &install_target,
            dry_run,
            "portinstall::install_main",
            |session| {
                let (target, record) = registration(session)?;
                let mut archive = package(session, &target)?;
                pending = Some(self.register(&mut archive, item, &target, record)?);
                Ok(())
            },
        )?;
        let activate_target = session.scalar("org.macports.activate")?;
        let mut image = None;
        let activate = session.run_prepared_target_with_native_body(
            &activate_target,
            dry_run,
            "portactivate::activate_main",
            |_| {
                let registered = pending.take().context(
                    "native activation needs a registered image; custom or skipped install requires registry image recovery",
                )?;
                image = Some(registered.activate_inner(false)?);
                Ok(())
            },
        )?;
        if image.is_some() {
            progress.phase(&format!(
                "Installed {} @{}{}",
                item.port.name,
                item.port.version(),
                item.variants
            ));
        }
        Ok(LifecycleResult {
            install,
            activate,
            image,
        })
    }
}
