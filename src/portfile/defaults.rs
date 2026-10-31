//! Lazy PortSystem defaults, resolved against global variables on every read.
use super::*;
use molt::{MoltResult, check_args, interp::ScalarChange};
const FORMATS: &[(&str, &str)] = &[
    ("use_7z", ".7z"),
    ("use_bzip2", ".tar.bz2"),
    ("use_dmg", ".dmg"),
    ("use_lzip", ".tar.lz"),
    ("use_lzma", ".tar.lzma"),
    ("use_tar", ".tar"),
    ("use_xz", ".tar.xz"),
    ("use_zip", ".zip"),
];
pub(super) fn install(interp: &mut Interp, id: ContextID) -> MoltResult {
    interp.set_global_scalar_hooks(id, read, changed);
    interp.add_context_command("default", command, id);
    for (name, template) in [
        ("distname", "${name}-${version}"),
        ("dist_subdir", "${name}"),
        ("archive.subdir", "${subport}"),
        ("archive_sites.listfile", "archive_sites.tcl"),
        ("archive_sites.listpath", "port1.0/fetch"),
        ("extract.suffix", ".tar.gz"),
        (
            "distfiles",
            "[list [join $distname][join ${extract.suffix}]]",
        ),
    ] {
        register(interp, id, name, template)?;
    }
    // Configuration/user options were populated before hooks were installed.
    for (name, _) in FORMATS {
        if interp.var_exists(&Value::from(*name)) {
            format_changed(interp, id, name, ScalarChange::Write)?;
        }
    }
    Ok(Value::empty())
}
fn command(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 3, "option value")?;
    interp.with_global_scope(|interp| register(interp, id, argv[1].as_str(), argv[2].as_str()))
}
pub(super) fn register(
    interp: &mut Interp,
    id: ContextID,
    name: &str,
    template: &str,
) -> MoltResult {
    if !interp.context::<State>(id).lazy_defaults.contains_key(name)
        && interp.var_exists(&Value::from(name))
    {
        return Ok(Value::empty());
    }
    interp.set_scalar(name, Value::from(template))?;
    interp
        .context::<State>(id)
        .lazy_defaults
        .insert(name.into(), template.into());
    Ok(Value::empty())
}
fn read(interp: &mut Interp, id: ContextID, name: &str) -> Result<Option<Value>, Exception> {
    let state = interp.context::<State>(id);
    let Some(template) = state.lazy_defaults.get(name).cloned() else {
        return Ok(None);
    };
    if !template.contains(['$', '[']) {
        return Ok(Some(Value::from(template)));
    }
    if !state.resolving_defaults.insert(name.into()) {
        return Err(error(format!("cyclic default while reading {name}")));
    }
    let result = interp.subst_global(&template);
    interp.context::<State>(id).resolving_defaults.remove(name);
    result.map(Some)
}
fn changed(
    interp: &mut Interp,
    id: ContextID,
    name: &str,
    change: ScalarChange,
) -> Result<(), Exception> {
    interp.context::<State>(id).lazy_defaults.remove(name);
    if FORMATS.iter().any(|(option, _)| *option == name)
        && !interp.context::<State>(id).changing_format
    {
        interp.with_global_scope(|interp| format_changed(interp, id, name, change))?;
    }
    Ok(())
}
fn format_changed(
    interp: &mut Interp,
    id: ContextID,
    name: &str,
    change: ScalarChange,
) -> MoltResult {
    interp.context::<State>(id).changing_format = true;
    let result = (|| {
        if change == ScalarChange::Write && interp.scalar(name)?.as_bool().unwrap_or(false) {
            for (other, _) in FORMATS {
                if *other != name {
                    interp.unset(other);
                }
            }
            let suffix = FORMATS
                .iter()
                .find(|(option, _)| *option == name)
                .unwrap()
                .1;
            register(interp, id, "extract.suffix", suffix)?;
        } else {
            let mut any = false;
            for (option, _) in FORMATS {
                if interp.var_exists(&Value::from(*option))
                    && interp.scalar(option)?.as_bool().unwrap_or(false)
                {
                    any = true;
                    break;
                }
            }
            if !any {
                register(interp, id, "extract.suffix", ".tar.gz")?;
            }
        }
        Ok(Value::empty())
    })();
    interp.context::<State>(id).changing_format = false;
    result
}
