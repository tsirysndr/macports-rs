//! Archive-fetch phase dispatch; URL preparation and transport remain host-owned.
use super::*;
use molt::{MoltResult, check_args};
pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portarchivefetch::checkfiles", check_files, id);
    interp.add_context_command("portarchivefetch::_archivefetch_start", start_quiet, id);
    interp.add_context_command("portarchivefetch::archivefetch_start", start, id);
    interp.add_context_command("portarchivefetch::archivefetch_main", main, id);
}
fn call(interp: &mut Interp, command: &str, args: &[&str]) -> MoltResult {
    interp.eval_value(&Value::from(
        std::iter::once(command)
            .chain(args.iter().copied())
            .map(Value::from)
            .collect::<Vec<_>>(),
    ))
}
fn enabled(interp: &mut Interp, name: &str) -> bool {
    interp
        .scalar(name)
        .ok()
        .and_then(|v| v.as_bool().ok())
        .unwrap_or(false)
}
fn has_files(interp: &mut Interp) -> Result<bool, Exception> {
    if !interp.var_exists(&"all_archive_files".into()) {
        return Ok(false);
    }
    Ok(!interp.scalar("all_archive_files")?.as_list()?.is_empty())
}
fn start(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    run_start(interp, id, argv, false)
}
fn start_quiet(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "quiet")?;
    run_start(interp, id, argv, argv[1].as_bool()?)
}
fn run_start(interp: &mut Interp, id: ContextID, argv: &[Value], quiet: bool) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        let binary = enabled(interp, "ports_binary_only");
        if !enabled(interp, "ports_source_only")
            && (binary || !call(interp, "portarchivefetch::check_destroot_done", &[])?.as_bool()?)
        {
            call(
                interp,
                "portarchivefetch::checkfiles",
                &["::portarchivefetch::archivefetch_urls"],
            )?;
        }
        if has_files(interp)? {
            if !call(interp, "portarchivefetch::archives_present", &[])?.as_bool()? {
                call(interp, "portarchivefetch::start_pings", &[])?;
            }
            if !quiet {
                phase_start::notice(interp, id, argv, "Fetching archive for")?;
            }
        } else if binary {
            return Err(error(
                "Binary-only mode requested with no usable archive sites configured",
            ));
        }
        call(interp, "portfetch::check_dns", &[])
    })
}
fn main(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        if has_files(interp)? {
            call(interp, "portarchivefetch::fetchfiles", &[])?;
        }
        Ok(0.into())
    })
}

fn check_files(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "urls")?;
    interp.with_global_scope(|interp| {
        let resource = call(interp, "portarchivefetch::get_full_archive_sites_path", &[])?;
        let mirrors = Value::from(vec![
            "::portarchivefetch::get_archive_site_urls".into(),
            resource,
        ]);
        let sites = Value::from(vec![
            "archive_sites".into(),
            Value::from(vec![Value::empty(), "ARCHIVE_SITE_LOCAL".into()]),
        ]);
        interp.eval_value(&Value::from(vec![
            "portfetch::checksites".into(),
            sites,
            mirrors,
        ]))
    })?;
    // Preserve the supplied variable in the caller's frame, as upvar does.
    archive_files::check_files(
        interp,
        id,
        &[
            "portarchivefetch::checkarchivefiles".into(),
            argv[1].clone(),
        ],
    )
}
