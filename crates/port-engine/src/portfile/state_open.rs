//! PortSystem's native state-file opening command.
use super::*;
use crate::build::{state::Compatibility, work_state};
use molt::MoltResult;
use std::{fs, io, os::unix::fs::symlink, path::Path};

fn enabled(interp: &mut Interp, name: &str) -> Result<bool, Exception> {
    Ok(interp.var_exists(&name.into()) && interp.scalar(name)?.as_bool()?)
}
fn call(interp: &mut Interp, words: &[&str]) -> MoltResult {
    interp.eval_value(&Value::from(
        words
            .iter()
            .map(|word| Value::from(*word))
            .collect::<Vec<_>>(),
    ))
}
pub(super) fn open(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    // Upstream accepts and ignores arbitrary arguments.
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        let work = interp.scalar("workpath")?.to_string();
        let subport = interp.scalar("subport")?.to_string();
        let portpath = interp.scalar("portpath")?.to_string();
        let dry = enabled(interp, "ports_dryrun")?;
        let ignore = enabled(interp, "ports_ignore_different")?;
        if !dry {
            if !Path::new(&work).join(".home").is_dir()
                || !Path::new(&work).join(".tmp").is_dir()
            {
                call(interp, &["portutil::create_workpath"])?;
            }
            if enabled(interp, "place_worksymlink")? {
                let link = interp.scalar("worksymlink")?.to_string();
                if !Path::new(&link).is_dir() {
                    if crate::privileges::dropped() {
                        crate::privileges::elevate("place_worksymlink").map_err(|err| error(err.to_string()))?;
                    }
                    match fs::remove_file(&link) {
                        Ok(()) => (),
                        Err(err) if err.kind() == io::ErrorKind::NotFound => (),
                        Err(err) => return Err(filesystem::path_error(&link, "could not replace work link", err)),
                    }
                    symlink(&work, &link).map_err(|err| filesystem::path_error(&link, "could not create work link", err))?;
                }
            }
        }
        // De-escalate to the build account if started as root.
        let user = build_user(interp);
        crate::privileges::drop_to(&user).map_err(|err| error(err.to_string()))?;
        let portfile = Path::new(&portpath).join("Portfile");
        let prepared = if dry {
            work_state::prepare_dry_run(Path::new(&work), &subport, &portfile, ignore)
        } else {
            let mask = interp.context::<State>(id).creation_mask;
            work_state::prepare(Path::new(&work), &subport, &portfile, mask, ignore)
        }.map_err(|err| error(err.to_string()))?;
        match &prepared.compatibility {
            Compatibility::PortfileChanged if dry => eprintln!("Portfile for {subport} changed since last build but not discarding previous state (dry run)"),
            Compatibility::PortfileChanged => eprintln!("Portfile for {subport} changed since last build; discarding previous state."),
            Compatibility::UnsupportedVersion(version) => {
                eprintln!("Warning: Unsupported statefile version '{version}'");
                eprintln!("Warning: Please run 'port selfupdate' to update to the latest version of MacPorts");
            }
            Compatibility::Current => (),
        }
        build_state::attach(interp.context::<State>(id), prepared.state)
            .map(Value::from)
            .map_err(|err| error(err.to_string()))
    })
}
