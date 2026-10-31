//! Archive location/presence lookup; presence never authenticates cached bytes.
use super::*;
use molt::{MoltResult, check_args};
pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("get_portimage_path", image_path, id);
    interp.add_context_command("find_portarchive_path", find, id);
    interp.add_context_command("portarchivefetch::archives_present", present, id);
    interp.add_context_command("supportedArchiveTypes", types, id);
}
fn call(interp: &mut Interp, words: Vec<Value>) -> MoltResult {
    interp.eval_value(&Value::from(words))
}
fn types(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    // Native archive decoding currently supports only bzip2-compressed tar.
    Ok(list(["tbz2".into()]))
}
fn image_path(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| {
        let filename = call(interp, vec!["get_portimage_name".into()])?;
        let path = paths::join([
            interp.scalar("portdbpath")?.as_str(),
            "software",
            interp.scalar("subport")?.as_str(),
            filename.as_str(),
        ]);
        file_normalize::command(&["file".into(), "normalize".into(), path.into()])
    })
}
fn find(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 2, "?include_installed?")?;
    let include = argv.get(1).map(Value::as_bool).transpose()?.unwrap_or(true);
    interp.with_global_scope(|interp| {
        let installed = if include {
            let mut args = vec!["registry_exists".into()];
            for key in ["subport", "version", "revision", "portvariants"] {
                args.push(interp.scalar(key)?);
            }
            call(interp, args)?.as_bool()?
        } else {
            false
        };
        let filename = call(interp, vec!["get_portimage_name".into()])?.to_string();
        let root = filename
            .rsplit_once('.')
            .map_or(filename.as_str(), |(root, _)| root);
        let base = if installed {
            paths::join([
                interp.scalar("portdbpath")?.as_str(),
                "software",
                interp.scalar("subport")?.as_str(),
            ])
        } else {
            paths::join([interp.scalar("portdbpath")?.as_str(), "incoming/verified"])
        };
        for kind in call(interp, vec!["supportedArchiveTypes".into()])?
            .as_list()?
            .iter()
        {
            let path = paths::join([base.as_str(), &format!("{root}.{kind}")]);
            if filesystem::predicate("isfile", &path) {
                return Ok(path.into());
            }
        }
        Ok(Value::empty())
    })
}
fn present(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| {
        let include = !interp
            .scalar("force_archive_refresh")
            .ok()
            .and_then(|v| v.as_bool().ok())
            .unwrap_or(false);
        let existing = call(interp, vec!["find_portarchive_path".into(), include.into()])?;
        let mut exists = !existing.as_str().is_empty();
        if !exists && include {
            let path = call(interp, vec!["get_portimage_path".into()])?.to_string();
            let root = path
                .rsplit_once('.')
                .map_or(path.as_str(), |(root, _)| root);
            exists = filesystem::predicate("isdirectory", root);
        }
        let urls = interp
            .scalar("portarchivefetch::archivefetch_urls")?
            .as_list()?;
        for pair in urls.chunks(2) {
            let archive = pair.get(1).map(Value::as_str).unwrap_or("");
            let path = format!("{}/{archive}", interp.scalar("archivefetch.fulldestpath")?);
            if !filesystem::predicate("isfile", &path) && !exists {
                return Ok(false.into());
            }
        }
        Ok(true.into())
    })
}
