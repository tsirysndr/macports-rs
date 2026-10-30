//! Native non-root destroot startup, before Portfile pre-destroot hooks.
use super::*;
use molt::MoltResult;

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portdestroot::destroot_start", start, id);
    interp.add_context_command("portstartupitem::startupitem_create", startup, id);
    for name in [
        "startupitems",
        "startupitem.create",
        "startupitem.custom_file",
        "startupitem.type",
    ] {
        commands::register(interp, id, name);
    }
}
fn call(interp: &mut Interp, words: Vec<Value>) -> MoltResult {
    interp.eval_value(&Value::from(words))
}
// Until launchd generation is implemented, detect requested startup items before
// deleting any previous stage. Ports without startup items need no generation.
fn optional(
    interp: &mut Interp,
    name: &str,
) -> std::result::Result<Option<Value>, molt::Exception> {
    if interp.var_exists(&name.into()) {
        interp.scalar(name).map(Some)
    } else {
        Ok(None)
    }
}
fn check_startup(interp: &mut Interp, id: ContextID) -> MoltResult {
    let items = optional(interp, "startupitems")?;
    let kind = optional(interp, "startupitem.type")?;
    let create = optional(interp, "startupitem.create")?
        .map(|v| v.as_bool())
        .transpose()?
        .unwrap_or(false);
    let custom = optional(interp, "startupitem.custom_file")?;
    let multiple = items.is_some_and(|v| !v.as_str().is_empty());
    let requested = create || custom.is_some_and(|v| !v.as_str().is_empty());
    if multiple || (requested && !kind.is_some_and(|v| v.as_str() == "none")) {
        interp
            .context::<State>(id)
            .unsupported
            .insert("startup item generation".into());
        return Err(error("native startup item generation is not implemented"));
    }
    Ok(Value::empty())
}
fn startup(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| check_startup(interp, id))
}
fn start(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    phase_start::notice(interp, id, argv, "Staging")?;
    interp.with_global_scope(|interp| {
        if unsafe { libc::getuid() == 0 || libc::geteuid() == 0 } {
            interp.context::<State>(id).unsupported.insert("destroot privilege lifecycle".into());
            return Err(error("privileged destroot startup requires the privilege worker"));
        }
        if interp.scalar("destroot.asroot")?.as_bool()? {
            return Err(error("You cannot run this port without root privileges. You need to re-run with 'sudo port'."));
        }
        check_startup(interp, id)?;
        let destroot = interp.scalar("destroot")?;
        let clean = interp.scalar("destroot.clean")?.as_str() == "yes";
        let mask = interp.scalar("destroot.umask")?;
        let old = call(interp, vec!["umask".into(), mask])?;
        let result = (|| {
            interp.set_scalar("portdestroot::oldmask", old.clone())?;
            if clean { call(interp, vec!["file".into(), "delete".into(), "-force".into(), "--".into(), destroot.clone()])?; }
            call(interp, vec!["file".into(), "mkdir".into(), destroot])?;
            StageContext { interp, id }.prepare_destroot_hierarchy().map_err(|err| error(err.to_string()))?;
            call(interp, vec!["portstartupitem::startupitem_create".into()])?;
            Ok(Value::empty())
        })();
        if result.is_err() {
            let _ = call(interp, vec!["umask".into(), old]);
        }
        result
    })
}
