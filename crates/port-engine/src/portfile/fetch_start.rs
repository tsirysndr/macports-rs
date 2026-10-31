//! Source fetch startup: cache short-circuit and distribution directory setup.
use super::*;
use molt::{MoltResult, check_args};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portfetch::fetch_start", start, id);
    interp.add_context_command("portfetch::_fetch_start", prepare, id);
}
fn call(interp: &mut Interp, words: &[&str]) -> MoltResult {
    interp.eval_value(&Value::from(
        words
            .iter()
            .map(|word| Value::from(*word))
            .collect::<Vec<_>>(),
    ))
}
fn start(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        if interp
            .scalar("portarchivefetch::attempted")
            .ok()
            .and_then(|v| v.as_bool().ok())
            .unwrap_or(false)
        {
            let name = interp.scalar("subport")?;
            let prefix = interp.scalar("UI_PREFIX")?;
            interp
                .context::<State>(id)
                .progress
                .clone()
                .unwrap_or_else(|| crate::progress::Progress::new(false))
                .phase_with_prefix(
                    prefix.as_str(),
                    &format!("Archive not available for {name}, building locally"),
                );
        }
        phase_start::notice(interp, id, argv, "Fetching distfiles for")?;
        if call(interp, &["portfetch::files_present"])?.as_bool()? {
            return Ok(0.into());
        }
        call(interp, &["portfetch::_fetch_start"])
    })
}
fn prepare(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        if interp.var_exists(&"all_dist_files".into()) {
            call(interp, &["portfetch::start_pings"])?;
        }
        let directory = interp.scalar("distpath")?;
        let path = directory.as_str();
        if !filesystem::predicate("isdirectory", path) && call(interp, &["file", "mkdir", path]).is_err() {
            call(interp, &["elevateToRoot", "fetch"])?;
            if let Err(err) = call(interp, &["file", "mkdir", path]) {
                return Err(error(format!("Unable to create distribution files path: {}", err.value())));
            }
            call(interp, &["chownAsRoot", path])?;
            call(interp, &["dropPrivileges"])?;
        }
        if !filesystem::predicate("owned", path)
            && let Err(err) = call(interp, &["chownAsRoot", path])
        {
            if filesystem::predicate("writable", path) {
                let prefix = interp.scalar("UI_PREFIX")?;
                call(interp, &["ui_warn", &format!("{prefix} Couldn't change ownership of distribution files path to macports user: {}", err.value())])?;
            } else {
                return Err(error(format!("Distribution files path {path} not writable and could not be chowned: {}", err.value())));
            }
        }
        call(interp, &["portfetch::check_dns"])
    })
}
