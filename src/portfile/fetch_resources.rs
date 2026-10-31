//! Selected-tree source mirror resources, evaluated in an isolated interpreter.
use super::*;
use molt::{MoltResult, check_args};
pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portfetch::get_full_mirror_sites_path", path, id);
    interp.add_context_command("portfetch::get_mirror_site_urls", urls, id);
}
fn path(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| {
        let Some(root) = interp
            .context::<State>(id)
            .groups
            .resource_directory
            .clone()
        else {
            return commands::unsupported(interp, id, argv);
        };
        let directory = interp.scalar("mirror_sites.listpath")?;
        let filename = interp.scalar("mirror_sites.listfile")?;
        let path = root.join(directory.as_str()).join(filename.as_str());
        Ok(path
            .to_str()
            .ok_or_else(|| error("mirror resource path is not UTF-8"))?
            .into())
    })
}
fn urls(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 3, "mirrorfile site")?;
    interp.with_global_scope(|interp| {
        let path = argv[1].as_str();
        if !interp
            .context::<State>(id)
            .mirror_resources
            .contains_key(path)
        {
            let platform = interp.scalar("os.platform")?;
            let major = interp.scalar("os.major")?.as_int()?;
            let major =
                u32::try_from(major).map_err(|_| error("invalid mirror OS major version"))?;
            // Both resource formats use portfetch::mirror_sites::sites. Reuse
            // the bounded, no-I/O interpreter without archive layout filtering.
            let resource = crate::install::sites::resource::Resource::read(
                std::path::Path::new(path),
                platform.as_str(),
                major,
            )
            .map_err(|err| error(format!("mirror resource: {err:#}")))?;
            interp
                .context::<State>(id)
                .mirror_resources
                .insert(path.into(), resource);
        }
        interp.context::<State>(id).mirror_resources[path]
            .urls(argv[2].as_str())
            .map(list)
            .map_err(|err| error(err.to_string()))
    })
}
