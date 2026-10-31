//! Activation callbacks; the host still owns registry activation and load targets.
use super::*;
use molt::{MoltResult, check_args};

impl Session {
    /// Drain notes queued after successful activation, for the CLI to display.
    pub fn take_notifications(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.interp.context::<State>(self.id).notifications)
    }
}
pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portactivate::activate_start", start, id);
    interp.add_context_command("portdeactivate::deactivate_start", start, id);
    interp.add_context_command("portuninstall::uninstall_start", start, id);
    interp.add_context_command("portactivate::activate_finish", finish, id);
    interp.add_context_command("ui_notifications_append", append, id);
}
fn start(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        let prefix = interp.scalar("prefix_frozen")?;
        if !filesystem::predicate("writable", prefix.as_str()) || crate::privileges::dropped() {
            crate::privileges::elevate(argv[0].as_str()).map_err(|err| error(err.to_string()))?;
        }
        Ok(Value::empty())
    })
}
fn finish(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        let subport = interp.scalar("subport")?;
        if interp
            .scalar("startupitem_autostart")
            .ok()
            .and_then(|v| v.as_bool().ok())
            .unwrap_or(false)
        {
            interp.set_scalar("portstartupitem::autostart_only", "yes".into())?;
            let result = interp.eval_value(&Value::from(vec![
                Value::from("eval_targets"),
                "load".into(),
            ]));
            interp.unset_var(&"portstartupitem::autostart_only".into());
            if result?.as_bool()? {
                interp
                    .context::<State>(id)
                    .progress
                    .clone()
                    .unwrap_or_else(|| crate::progress::Progress::new(false))
                    .message(&format!("Error: Failed to load {subport}"));
                return Ok(1.into());
            }
        }
        if let Ok(notes) = interp.element("PortInfo", "notes")
            && !notes.as_list()?.is_empty()
        {
            interp.eval_value(&Value::from(vec![
                "ui_notifications_append".into(),
                subport,
                notes,
            ]))?;
        }
        Ok(0.into())
    })
}
fn append(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 3, "port notes")?;
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp
        .context::<State>(id)
        .notifications
        .push((argv[1].to_string(), argv[2].to_string()));
    Ok(Value::empty())
}
