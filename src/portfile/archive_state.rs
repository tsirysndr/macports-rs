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
