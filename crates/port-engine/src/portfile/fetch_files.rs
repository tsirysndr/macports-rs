//! Native distfile/patch candidate bookkeeping and fetch initialization.
use super::*;
use molt::{MoltResult, check_args};
fn split(file: &str) -> (&str, &str) {
    if let Some((name, tag)) = file.rsplit_once(':')
        && !name.is_empty()
        && !tag.is_empty()
        && tag
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        (name, tag)
    } else {
        (file, "")
    }
}
pub(super) fn install(interp: &mut Interp, id: ContextID) -> MoltResult {
    for name in ["getdisttag", "getdistname"] {
        interp.add_context_command(name, distpart, id);
    }
    for name in ["portfetch::checkpatchfiles", "portfetch::checkdistfiles"] {
        interp.add_context_command(name, candidates, id);
    }
    interp.add_context_command("portfetch::checkfiles", check, id);
    interp.add_context_command("portfetch::fetch_init", init, id);
    interp.set_scalar("portfetch::fetch_urls", Value::empty())?;
    Ok(Value::empty())
}
fn distpart(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "filename")?;
    let (name, tag) = split(argv[1].as_str());
    Ok(if argv[0].as_str() == "getdisttag" {
        tag
    } else {
        name
    }
    .into())
}
fn candidates(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "urls")?;
    let patch = argv[0].as_str().ends_with("checkpatchfiles");
    let files = interp.with_global_scope(|interp| -> Result<Vec<Value>, Exception> {
        let option = if patch { "patchfiles" } else { "distfiles" };
        if !interp.var_exists(&option.into()) {
            return Ok(vec![]);
        }
        Ok(interp.scalar(option)?.as_list()?.to_vec())
    })?;
    for file in files {
        let pair =
            interp.with_global_scope(|interp| -> Result<Option<(Value, Value)>, Exception> {
                let path = format!("{}/{}", interp.scalar("filespath")?, file);
                if filesystem::predicate("exists", &path) {
                    return Ok(None);
                }
                let (name, tag) = split(file.as_str());
                let site = if !tag.is_empty() {
                    tag
                } else if patch && interp.var_exists(&"patch_sites".into()) {
                    "patch_sites"
                } else {
                    "master_sites"
                };
                let mut all = if interp.var_exists(&"all_dist_files".into()) {
                    interp.scalar("all_dist_files")?.as_list()?.to_vec()
                } else {
                    vec![]
                };
                all.push(name.into());
                interp.set_scalar("all_dist_files", all.into())?;
                Ok(Some((site.into(), name.into())))
            })?;
        if let Some((site, name)) = pair {
            let mut urls = if interp.var_exists(&argv[1]) {
                interp.var(&argv[1])?.as_list()?.to_vec()
            } else {
                vec![]
            };
            urls.extend([site, name]);
            interp.set_var(&argv[1], urls.into())?;
        }
    }
    Ok(Value::empty())
}
fn check(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "urls")?;
    interp.with_global_scope(|interp| {
        let disabled = interp
            .scalar("ports_fetch_no-mirrors")
            .is_ok_and(|v| v.as_str() != "no");
        let nomirror = !disabled
            && interp
                .scalar("license")?
                .as_list()?
                .iter()
                .any(|v| v.as_str().eq_ignore_ascii_case("nomirror"));
        let extras = |site: &str, global: &Value| Value::from(vec![global.clone(), site.into()]);
        let (patch, master) = if disabled || nomirror {
            (Value::empty(), Value::empty())
        } else {
            let global = interp.scalar("global_mirror_site")?;
            (
                extras("PATCH_SITE_LOCAL", &global),
                extras("MASTER_SITE_LOCAL", &global),
            )
        };
        let resource = interp.eval_value(&Value::from(vec![Value::from(
            "portfetch::get_full_mirror_sites_path",
        )]))?;
        let command = Value::from(vec!["portfetch::get_mirror_site_urls".into(), resource]);
        interp.eval_value(&Value::from(vec![
            "portfetch::checksites".into(),
            Value::from(vec![
                "patch_sites".into(),
                patch,
                "master_sites".into(),
                master,
            ]),
            command,
        ]))
    })?;
    candidates(
        interp,
        id,
        &["portfetch::checkpatchfiles".into(), argv[1].clone()],
    )?;
    candidates(
        interp,
        id,
        &["portfetch::checkdistfiles".into(), argv[1].clone()],
    )
}
fn init(interp: &mut Interp, _: ContextID, _: &[Value]) -> MoltResult {
    interp.with_global_scope(|interp| {
        if interp.scalar("portfetch::fetch_urls")?.as_str().is_empty() {
            interp.eval_value(&Value::from(vec![
                "portfetch::checkfiles".into(),
                "::portfetch::fetch_urls".into(),
            ]))?;
        }
        Ok(Value::empty())
    })
}
