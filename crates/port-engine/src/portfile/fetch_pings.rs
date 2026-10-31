//! Probe candidate mirrors and wait for the upstream-selected primary mirror.
use super::*;
use molt::{MoltResult, check_args};
pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portfetch::start_pings", source, id);
    interp.add_context_command("portarchivefetch::start_pings", archive, id);
}
fn call(interp: &mut Interp, command: &str, arguments: Vec<Value>) -> MoltResult {
    let mut words = vec![command.into()];
    words.extend(arguments);
    interp.eval_value(&Value::from(words))
}
fn candidates(interp: &mut Interp, variable: &str) -> MoltResult {
    let candidates = interp.scalar(variable)?.as_list()?;
    for pair in candidates.chunks(2) {
        let key = format!("portfetch::urlmap({})", pair[0]);
        if interp.var_exists(&key.clone().into()) {
            let urls = interp.var(&key.into())?;
            call(interp, "async_ping_start", vec![urls])?;
        }
    }
    Ok(Value::empty())
}
fn wait_first(interp: &mut Interp, urls: Value) -> MoltResult {
    let urls = urls.as_list()?;
    if let Some(first) = urls.first().filter(|url| !url.as_str().is_empty()) {
        call(interp, "wait_for_pingtime", vec![first.clone()])?;
    }
    Ok(Value::empty())
}
fn source(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        candidates(interp, "portfetch::fetch_urls")?;
        if interp
            .scalar("license")?
            .as_list()?
            .iter()
            .any(|v| v.as_str().eq_ignore_ascii_case("nomirror"))
            || (interp.var_exists(&"ports_fetch_no-mirrors".into())
                && interp.scalar("ports_fetch_no-mirrors")?.as_str() != "no")
        {
            return Ok(Value::empty());
        }
        let path = call(interp, "portfetch::get_full_mirror_sites_path", vec![])?;
        let class = interp.scalar("global_mirror_site")?;
        let urls = call(interp, "portfetch::get_mirror_site_urls", vec![path, class])?;
        wait_first(interp, urls)
    })
}
fn archive(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        candidates(interp, "portarchivefetch::archivefetch_urls")?;
        let sites = interp.scalar("archive_sites")?.as_list()?;
        let class = if sites
            .iter()
            .any(|site| site.as_str().starts_with("macports_archives::"))
        {
            "macports_archives"
        } else {
            sites
                .first()
                .map(|v| v.as_str().split(':').next().unwrap_or(""))
                .unwrap_or("")
        };
        let path = call(
            interp,
            "portarchivefetch::get_full_archive_sites_path",
            vec![],
        )?;
        let urls = call(
            interp,
            "portarchivefetch::get_archive_site_urls",
            vec![path, class.into()],
        )?;
        wait_first(interp, urls)
    })
}
