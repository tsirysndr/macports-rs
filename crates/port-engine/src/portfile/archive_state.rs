//! State checks shared by archive fetch startup and source-build selection.
use super::*;
use molt::{MoltResult, check_args};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portarchivefetch::check_destroot_done", destroot_done, id);
}
fn destroot_done(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        let destroot = interp.scalar("destroot")?;
        if !filesystem::predicate("isdirectory", destroot.as_str()) {
            return Ok(false.into());
        }
        let existing = interp.scalar("target_state_fd").ok();
        let owned = existing.is_none();
        let handle = match existing {
            Some(handle) => handle,
            None => state_open::open(interp, id, &["open_statefile".into()])?,
        };
        let result = (|| -> MoltResult {
            if owned {
                interp.set_scalar("target_state_fd", handle.clone())?;
            }
            let state = interp
                .context::<State>(id)
                .build_states
                .get_mut(handle.as_str())
                .ok_or_else(|| error(format!("unknown build-state handle \"{handle}\"")))?;
            Ok(state
                .contains("target", "org.macports.destroot")
                .map_err(|err| error(err.to_string()))?
                .into())
        })();
        if owned {
            interp
                .context::<State>(id)
                .build_states
                .remove(handle.as_str());
            interp.unset_var(&"target_state_fd".into());
        }
        result
    })
}

impl Session {
    /// Accept an authenticated archive for this live build and mark the source
    /// phases satisfied. The archive bytes, plan identity, and live coordinates
    /// are checked before writing any completion records. The host retains the
    /// verified archive for installation and cancels any pending source fetch.
    pub fn accept_verified_archive(
        &mut self,
        archive: &mut crate::install::archive::VerifiedArchive,
        item: &crate::install::Item<'_>,
        target: &crate::install::target::Target,
        limits: crate::install::inspect::Limits,
    ) -> Result<()> {
        check_supported(&mut self.interp, self.id)?;
        ensure!(
            self.interp
                .context::<State>(self.id)
                .system_executor
                .is_some(),
            "archive acceptance requires an execution-enabled session"
        );
        for option in ["ports_dryrun", "ports_source_only"] {
            let enabled = self
                .interp
                .scalar(option)
                .ok()
                .and_then(|v| v.as_bool().ok())
                .unwrap_or(false);
            ensure!(!enabled, "archive acceptance is disabled by {option}");
        }
        ensure!(
            item.variants_resolved && item.provider_fallbacks.is_empty(),
            "archive acceptance requires resolved variants and dependency providers"
        );
        let fd = self.scalar("target_state_fd")?;
        ensure!(
            self.interp
                .context::<State>(self.id)
                .build_states
                .contains_key(&fd),
            "unknown build-state handle \"{fd}\""
        );
        let contents = crate::install::inspect::tbz2(archive, limits)?.contents;
        contents.matches(item)?;
        target.matches(&contents)?;
        ensure!(
            self.scalar("subport")?
                .eq_ignore_ascii_case(&item.port.name)
                && self.scalar("version")? == item.port.get("version")
                && self.scalar("revision")?.parse::<u64>()? == contents.revision
                && self.scalar("epoch")?.parse::<u64>()? == contents.epoch
                && self.scalar("portvariants")? == item.variants
                && self.active_variants()? == item.variants,
            "live Portfile identity changed before archive acceptance"
        );
        ensure!(
            self.invoke("_get_archive_platform", &[])? == target.platform()
                && crate::tcl_list::parse(&self.invoke("get_canonical_archs", &[])?)?
                    == target.architectures(),
            "live archive coordinates changed before archive acceptance"
        );
        let state = self
            .interp
            .context::<State>(self.id)
            .build_states
            .get_mut(&fd)
            .expect("validated owned state handle");
        for phase in [
            "archivefetch",
            "fetch",
            "checksum",
            "extract",
            "patch",
            "configure",
            "build",
            "destroot",
        ] {
            state.record("target", &format!("org.macports.{phase}"))?;
        }
        Ok(())
    }
}
