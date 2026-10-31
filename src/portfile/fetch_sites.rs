//! Native mirror expansion and URL-map construction for fetch/archivefetch.
use super::*;
use crate::install::sites::requests::resolve_mirror;
use molt::{MoltResult, check_args};
use std::sync::OnceLock;
fn tagged(site: &str) -> Option<(&str, &str)> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let captures = RE
        .get_or_init(|| regex::Regex::new(r"([a-zA-Z]+://.+/?):([0-9A-Za-z_-]+)$").unwrap())
        .captures(site)?;
    Some((captures.get(1)?.as_str(), captures.get(2)?.as_str()))
}
fn url(site: &str) -> Option<&str> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"[a-zA-Z]+://.+").unwrap())
        .find(site)
        .map(|m| m.as_str())
}
pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portfetch::checksites", check, id);
    interp.add_context_command("portfetch::mirror_sites", mirrors, id);
    interp.add_context_command("portfetch::resolve_env_site", environment, id);
}
fn expand(
    interp: &mut Interp,
    id: ContextID,
    class: &str,
    tag: &str,
    subdir: &str,
    command: &Value,
) -> MoltResult {
    let mut words = command.as_list()?.to_vec();
    words.push(class.into());
    let sites = interp.eval_value(&Value::from(words))?;
    if sites.as_str().is_empty() {
        if class != interp.scalar("global_mirror_site")?.as_str() {
            interp
                .context::<State>(id)
                .progress
                .clone()
                .unwrap_or_else(|| crate::progress::Progress::new(false))
                .warning(&format!("No mirror sites on file for class {class}"));
        }
        return Ok(Value::empty());
    }
    let name = interp.scalar("name")?;
    let dist = interp.scalar("dist_subdir")?;
    let mut result = Vec::new();
    for element in sites.as_list()?.iter() {
        let mut resolved = resolve_mirror(element.as_str(), subdir, name.as_str(), dist.as_str());
        if !tag.is_empty() {
            resolved.push(':');
            resolved.push_str(tag);
        }
        result.push(resolved);
    }
    Ok(list(result))
}
fn mirrors(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 5, 5, "mirrors tag subdir mirrorscmd")?;
    interp.with_global_scope(|interp| {
        expand(
            interp,
            id,
            argv[1].as_str(),
            argv[2].as_str(),
            argv[3].as_str(),
            &argv[4],
        )
    })
}
fn resolve_env(interp: &mut Interp, site: &str) -> Result<(String, bool), Exception> {
    if let Some((element, tag)) = tagged(site) {
        let two = tagged(element).is_some();
        let name = interp.scalar("name")?;
        let dist = interp.scalar("dist_subdir")?;
        let mut resolved = resolve_mirror(
            if two { element } else { site },
            "",
            name.as_str(),
            dist.as_str(),
        );
        if two {
            resolved.push(':');
            resolved.push_str(tag);
        }
        Ok((resolved, two))
    } else {
        Ok((site.into(), false))
    }
}
fn environment(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "site")?;
    interp.with_global_scope(|interp| {
        let (site, tagged) = resolve_env(interp, argv[1].as_str())?;
        Ok(Value::from(vec![site.into(), tagged.into()]))
    })
}
fn append(interp: &mut Interp, key: &str, value: Value) -> MoltResult {
    let mut items = interp
        .element("portfetch::urlmap", key)
        .ok()
        .map(|v| v.as_list().map(|v| v.to_vec()))
        .transpose()?
        .unwrap_or_default();
    items.push(value);
    interp.set_element_return("portfetch::urlmap", key, items.into())
}
fn check(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 3, "sitelists mirrorscmd")?;
    interp.with_global_scope(|interp| {
        for pair in argv[1].as_list()?.chunks(2) {
            let listname = pair[0].as_str();
            if !interp.var_exists(&pair[0]) {
                continue;
            }
            let mut full = interp.scalar(listname)?.as_list()?.to_vec();
            let extras = pair
                .get(1)
                .cloned()
                .unwrap_or_else(Value::empty)
                .as_list()?;
            let mut global_sites = Vec::new();
            let mut untagged = Vec::new();
            if extras.len() >= 2 {
                if let Ok(env) = interp.element("env", extras[1].as_str()) {
                    let mut local = Vec::new();
                    for site in env.as_list()?.iter() {
                        let (site, tagged) = resolve_env(interp, site.as_str())?;
                        if !tagged {
                            untagged.push(Value::from(site.clone()));
                        }
                        local.push(site.into());
                    }
                    local.extend(full);
                    full = local;
                }
                if !extras[0].as_str().is_empty() {
                    full.insert(0, extras[0].clone());
                    global_sites = expand(interp, id, extras[0].as_str(), "", "", &argv[2])?
                        .as_list()?
                        .to_vec();
                }
            }
            let mut sites = Vec::new();
            for site in full {
                if let Some(url) = url(site.as_str()) {
                    sites.push(Value::from(url));
                } else {
                    let parts: Vec<_> = site.as_str().split(':').collect();
                    if parts.len() > 3 {
                        interp
                            .context::<State>(id)
                            .progress
                            .clone()
                            .unwrap_or_else(|| crate::progress::Progress::new(false))
                            .message(&format!(
                                "Error: Unable to process mirror sites for: {site}, ignoring."
                            ));
                    }
                    let mut subdir = parts.get(1).copied().unwrap_or("").to_owned();
                    if let Ok(suffix) = interp.scalar(&format!("{listname}.mirror_subdir")) {
                        subdir.push_str(suffix.as_str());
                    }
                    sites.extend(
                        expand(
                            interp,
                            id,
                            parts[0],
                            parts.get(2).copied().unwrap_or(""),
                            &subdir,
                            &argv[2],
                        )?
                        .as_list()?
                        .iter()
                        .cloned(),
                    );
                }
            }
            let mut tags = BTreeSet::new();
            for site in sites {
                if let Some((base, tag)) = tagged(site.as_str()) {
                    append(interp, tag, base.into())?;
                    tags.insert(tag.to_owned());
                } else {
                    append(interp, listname, site)?;
                }
            }
            for tag in tags {
                let mut sites = global_sites.clone();
                sites.extend(untagged.iter().cloned());
                sites.extend(
                    interp
                        .element("portfetch::urlmap", &tag)?
                        .as_list()?
                        .iter()
                        .cloned(),
                );
                interp.set_element("portfetch::urlmap", &tag, sites.into())?;
            }
        }
        Ok(Value::empty())
    })
}
