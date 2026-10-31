//! Deferred PortSystem startup-item defaults and type selection.
use super::*;
use molt::check_args;

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portstartupitem::get_startupitem_type", kind, id);
}
pub(in crate::portfile) fn initialize(interp: &mut Interp, id: ContextID) -> MoltResult {
    for (name, value) in [
        ("autostart", "no"),
        ("custom_file", ""),
        ("debug", "no"),
        ("executable", ""),
        ("group", ""),
        ("init", ""),
        ("install", "$system_options(startupitem_install)"),
        ("location", "LaunchDaemons"),
        ("logevents", "no"),
        ("logfile", ""),
        ("logfile.stderr", "${startupitem.logfile}"),
        ("name", "${subport}"),
        ("netchange", "no"),
        ("pidfile", ""),
        ("plist", "${startupitem.uniquename}.plist"),
        ("requires", ""),
        ("restart", ""),
        ("start", ""),
        ("stop", ""),
        ("type", "[portstartupitem::get_startupitem_type]"),
        ("uniquename", "org.macports.${startupitem.name}"),
        ("user", ""),
        ("daemondo.verbosity", "1"),
    ] {
        let name = format!("startupitem.{name}");
        if !interp.var_exists(&name.clone().into()) {
            crate::portfile::defaults::register(interp, id, &name, value)?;
            interp.context::<State>(id).execution_defaults.insert(name);
        }
    }
    Ok(Value::empty())
}
fn kind(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| {
        let enabled = if interp.var_exists(&"startupitem.create".into()) {
            interp.scalar("startupitem.create")?.as_bool()?
        } else {
            false
        };
        if !enabled {
            return Ok("none".into());
        }
        let configured = interp.element("system_options", "startupitem_type")?;
        if configured.as_str().is_empty() || configured.as_str() == "default" {
            Ok(if interp.scalar("os.platform")?.as_str() == "darwin" {
                "launchd"
            } else {
                "none"
            }
            .into())
        } else {
            Ok(configured)
        }
    })
}
