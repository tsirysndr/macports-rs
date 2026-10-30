//! Callback execution after the host has prepared a target's runtime context.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetOutcome {
    Completed,
    AlreadyCompleted,
    DryRun,
}

impl Session {
    /// Execute one prepared dependency item's callbacks in PortSystem order.
    ///
    /// The host must first load its runtime package and arrange prerequisites,
    /// environment, cwd, sandbox/tracing, privileges, and log handling. This is
    /// the callback/state portion of target_run, not the full target driver.
    /// Stateful items require a live session-owned state handle. The caller
    /// retains that handle and must close it after the entire target lifecycle.
    pub fn execute_target(
        &mut self,
        item: &str,
        state_handle: Option<&str>,
        dry_run: bool,
    ) -> Result<TargetOutcome> {
        self.execute_target_inner(item, state_handle, dry_run, false)
    }

    pub(super) fn execute_target_inner(
        &mut self,
        item: &str,
        state_handle: Option<&str>,
        dry_run: bool,
        manage_developer: bool,
    ) -> Result<TargetOutcome> {
        check_supported(&mut self.interp, self.id)?;
        ensure!(
            self.interp
                .context::<State>(self.id)
                .system_executor
                .is_some(),
            "target execution requires an execution-enabled session"
        );
        let item = Value::from(item);
        let mut field = |key| ditems::field(&mut self.interp, self.id, &item, key).map_err(failure);
        let name = field("name")?.to_string();
        let procedure = field("procedure")?.to_string();
        let stateful = field("state")?.as_str() != "no";
        let init = field("init")?.to_string();
        ensure!(
            !procedure.is_empty(),
            "{name} does not have a registered procedure"
        );
        if stateful {
            let handle = state_handle
                .ok_or_else(|| anyhow::anyhow!("{name} requires a build-state handle"))?;
            ensure!(
                self.interp
                    .context::<State>(self.id)
                    .build_states
                    .contains_key(handle),
                "unknown build-state handle \"{handle}\""
            );
        }
        // Initialization runs even for completed targets and dry runs.
        if !init.is_empty() {
            self.invoke(&init, &[&name])?;
        }
        let stateful = ditems::field(&mut self.interp, self.id, &item, "state")
            .map_err(failure)?
            .as_str()
            != "no";
        if stateful
            && self.invoke(
                "check_statefile",
                &[
                    "target",
                    &name,
                    state_handle
                        .ok_or_else(|| anyhow::anyhow!("{name} requires a build-state handle"))?,
                ],
            )? == "1"
        {
            return Ok(TargetOutcome::AlreadyCompleted);
        }
        if dry_run && name != "org.macports.uninstall" {
            return Ok(TargetOutcome::DryRun);
        }
        // Read each callback list when its turn arrives: earlier callbacks can
        // modify later lists, just as in the upstream interpreter.
        let body_result = (|| -> Result<()> {
            self.target_callbacks(&item, "prerun", &name)?;
            if manage_developer {
                let developer = self.scalar("configure.developer_dir")?;
                self.interp
                    .set_element("env", "DEVELOPER_DIR", developer.into())
                    .map_err(failure)?;
            }
            self.target_callbacks(&item, "pre", &name)?;
            self.invoke(&procedure, &[&name])?;
            self.target_callbacks(&item, "post", &name)?;
            Ok(())
        })();
        if manage_developer {
            // Cleanup bypasses the unsupported-command guard: even a caught
            // unsupported operation must not leak target environment state.
            self.interp.unset_element("env", "DEVELOPER_DIR");
        }
        body_result?;
        self.target_callbacks(&item, "postrun", &name)?;
        let stateful = ditems::field(&mut self.interp, self.id, &item, "state")
            .map_err(failure)?
            .as_str()
            != "no";
        let always = ditems::field(&mut self.interp, self.id, &item, "runtype")
            .map_err(failure)?
            .as_str()
            == "always";
        if stateful && !always {
            self.invoke(
                "write_statefile",
                &[
                    "target",
                    &name,
                    state_handle
                        .ok_or_else(|| anyhow::anyhow!("{name} requires a build-state handle"))?,
                ],
            )?;
        }
        Ok(TargetOutcome::Completed)
    }
    fn target_callbacks(&mut self, item: &Value, key: &str, name: &str) -> Result<()> {
        let value = ditems::field(&mut self.interp, self.id, item, key).map_err(failure)?;
        let callbacks = if matches!(key, "pre" | "post") {
            value.as_list().map_err(failure)?.to_vec()
        } else if value.as_str().is_empty() {
            Vec::new()
        } else {
            vec![value]
        };
        for callback in callbacks {
            self.invoke(callback.as_str(), &[name])?;
        }
        Ok(())
    }
}
