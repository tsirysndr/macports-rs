//! Configured destroot command execution; preparation and cleanup are separate hooks.
use super::*;
use molt::MoltResult;

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    for name in [
        "destroot.destdir",
        "destroot.asroot",
        "destroot.clean",
        "destroot.keepdirs",
        "destroot.umask",
        "destroot.violate_mtree",
        "destroot.delete_la_files",
    ] {
        commands::register(interp, id, name);
    }
    interp.add_context_command("portdestroot::destroot_getargs", args, id);
    interp.add_context_command("portdestroot::destroot_main", main, id);
}
fn args(interp: &mut Interp, _: ContextID, _: &[Value]) -> MoltResult {
    interp.with_global_scope(|interp| build_phase::make_args(interp, "destroot"))
}
fn main(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        // This is initialized by the host's filesystem-cloning configuration.
        // An explicit empty value disables the copy-tool wrapper directory.
        let clonebin = interp.element("system_options", "clonebin_path")?;
        let saved_path = if clonebin.as_str().is_empty() {
            None
        } else {
            let path = interp.element("env", "PATH")?;
            interp.set_element("env", "PATH", format!("{clonebin}:{path}").into())?;
            Some(path)
        };
        let result =
            interp.eval_value(&Value::from(vec!["command_exec".into(), "destroot".into()]));
        // Like Tcl try/finally, restore the original PATH even on child failure.
        if let Some(path) = saved_path {
            interp.set_element("env", "PATH", path)?;
        }
        result?;
        Ok(0.into())
    })
}
