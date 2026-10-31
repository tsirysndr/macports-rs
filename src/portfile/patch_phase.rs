//! Native PortSystem patch target procedures.
use super::*;
use molt::{MoltResult, check_args};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portpatch::build_getpatchtype", patch_type, id);
    interp.add_context_command("portpatch::patch_main", main, id);
    commands::register(interp, id, "patch.asroot");
}

fn call(interp: &mut Interp, words: &[&str]) -> MoltResult {
    interp.eval_value(&Value::from(
        words
            .iter()
            .map(|word| Value::from(*word))
            .collect::<Vec<_>>(),
    ))
}

fn find(interp: &mut Interp, program: &str, hint: &str) -> MoltResult {
    let hint = interp.scalar(&format!("portutil::autoconf::{hint}"))?;
    call(interp, &["findBinary", program, hint.as_str()])
}

fn patch_type(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 0, "?args?")?;
    interp.with_global_scope(|interp| {
        if !interp.var_exists(&"patch.type".into()) {
            return find(interp, "patch", "patch_path");
        }
        let kind = interp.scalar("patch.type")?;
        if kind.as_str() == "gnu" {
            return find(interp, "gpatch", "gnupatch_path");
        }
        interp
            .context::<State>(id)
            .progress
            .clone()
            .unwrap_or_else(|| crate::progress::Progress::new(false))
            .warning(&format!("Unknown patch.type {kind}, using 'patch'"));
        find(interp, "patch", "patch_path")
    })
}

fn main(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        if !interp.var_exists(&"patchfiles".into()) {
            return Ok(0.into());
        }
        let patches = interp.scalar("patchfiles")?;
        if patches.as_str().is_empty() {
            return Ok(0.into());
        }
        phase_start::notice(interp, id, argv, "Applying patches to")?;
        let files = interp.scalar("filespath")?.to_string();
        let dist = interp.scalar("distpath")?.to_string();
        let mut list = Vec::new();
        for patch in patches.as_list()?.iter() {
            let name = call(interp, &["getdistname", patch.as_str()])?;
            let local = format!("{files}/{name}");
            let fetched = format!("{dist}/{name}");
            if filesystem::predicate("exists", &local) {
                list.push(local);
            } else if filesystem::predicate("exists", &fetched) {
                list.push(fetched);
            } else {
                return Err(error(format!("Patch file {patch} is missing")));
            }
        }
        let gzcat = format!("{} -dc", find(interp, "gzip", "gzip_path")?);
        let bzcat = format!("{} -dc", find(interp, "bzip2", "bzip2_path")?);
        let xzcat = find(interp, "xz", "xz_path")
            .ok()
            .map(|path| format!("{path} -dc"));
        for patch in list {
            let extension = std::path::Path::new(&patch)
                .extension()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default();
            let (before, after) = match extension.as_str() {
                "Z" | "gz" => (format!("{gzcat} \"{patch}\" | ("), ")".to_string()),
                "bz2" => (format!("{bzcat} \"{patch}\" | ("), ")".to_string()),
                "xz" => match &xzcat {
                    Some(xzcat) => (format!("{xzcat} \"{patch}\" | ("), ")".to_string()),
                    None => {
                        return Err(error(
                            "xz binary not found; port needs to add 'depends_patch bin:xz:xz'",
                        ));
                    }
                },
                _ => (String::new(), format!("< '{patch}'")),
            };
            call(interp, &["command_exec", "patch", &before, &after])?;
        }
        Ok(0.into())
    })
}
