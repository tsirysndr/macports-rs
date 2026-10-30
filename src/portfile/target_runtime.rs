//! Session-local environment and state-handle lifecycle for prepared targets.
use super::*;

impl Session {
    /// Run callbacks with PortSystem HOME/TMPDIR/DEVELOPER_DIR handling and
    /// automatic state-file ownership. The host must still load runtime
    /// packages and prepare dependencies, cwd, sandbox/tracing, privileges,
    /// and log handling. This does not enable ordinary CLI install execution.
    pub fn run_prepared_target(&mut self, item: &str, dry_run: bool) -> Result<TargetOutcome> {
        check_supported(&mut self.interp, self.id)?;
        ensure!(
            self.interp
                .context::<State>(self.id)
                .system_executor
                .is_some(),
            "target execution requires an execution-enabled session"
        );
        let stateful = ditems::field(&mut self.interp, self.id, &item.into(), "state")
            .map_err(failure)?
            .as_str()
            != "no";
        let work = self.scalar("workpath")?;
        let database = self.scalar("portdbpath")?;
        let saved_home = paths::join([database.as_str(), "home"]);
        let saved_dry = self.interp.scalar("ports_dryrun").ok();
        let mut handle = None;
        let result = (|| -> Result<TargetOutcome> {
            self.interp
                .set_element("env", "HOME", paths::join([work.as_str(), ".home"]).into())
                .map_err(failure)?;
            self.interp
                .set_element("env", "TMPDIR", paths::join([work.as_str(), ".tmp"]).into())
                .map_err(failure)?;
            self.interp
                .set_scalar("ports_dryrun", dry_run.into())
                .map_err(failure)?;
            if stateful {
                let fd = self.invoke("open_statefile", &[])?;
                handle = Some(fd.clone());
                self.interp
                    .set_scalar("target_state_fd", fd.into())
                    .map_err(failure)?;
            }
            self.execute_target_inner(item, handle.as_deref(), dry_run, true)
        })();
        // Always release the actual owned file, including after scripts poison
        // their session by catching an unsupported operation.
        if let Some(handle) = handle {
            self.interp
                .context::<State>(self.id)
                .build_states
                .remove(&handle);
            self.interp.unset_var(&"target_state_fd".into());
        }
        self.interp.unset_element("env", "TMPDIR");
        let home_result = self
            .interp
            .set_element("env", "HOME", saved_home.into())
            .map_err(failure);
        let dry_result = if let Some(value) = saved_dry {
            self.interp
                .set_scalar("ports_dryrun", value)
                .map_err(failure)
        } else {
            self.interp.unset_var(&"ports_dryrun".into());
            Ok(())
        };
        // Preserve the phase's original error if cleanup also fails.
        let outcome = result?;
        home_result?;
        dry_result?;
        Ok(outcome)
    }
}
