//! Native non-root destroot startup, before Portfile pre-destroot hooks.
use super::*;
use molt::MoltResult;

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portdestroot::destroot_start", start, id);
}

fn call(interp: &mut Interp, words: Vec<Value>) -> MoltResult {
    interp.eval_value(&Value::from(words))
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
        startupitem::preflight(interp, id)?;
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
