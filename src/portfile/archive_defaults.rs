//! Caller-provided archive metadata, resolved lazily during Portfile evaluation.
use super::*;
use molt::{MoltResult, check_args};
use std::rc::Rc;

type Resolver = dyn Fn(&str, &str) -> Result<Vec<String>>;

/// Resolves archive-site defaults for the live listpath/listfile values. The
/// caller captures the active tree, installation layout, and startup config.
#[derive(Clone)]
pub struct ArchiveSites {
    defaults: Rc<Resolver>,
    resources: Option<Rc<ResourceResolver>>,
}
type ResourceResolver =
    dyn Fn(&str, &str) -> Result<(String, Rc<crate::install::sites::Selection>)>;
impl ArchiveSites {
    pub fn new(resolve: impl Fn(&str, &str) -> Result<Vec<String>> + 'static) -> Self {
        Self {
            defaults: Rc::new(resolve),
            resources: None,
        }
    }
    /// Share selected resource metadata between defaults and runtime getters.
    pub fn with_resources(
        resolve: impl Fn(&str, &str) -> Result<(String, Rc<crate::install::sites::Selection>)> + 'static,
    ) -> Self {
        let resolve = Rc::new(resolve);
        let defaults = resolve.clone();
        Self {
            defaults: Rc::new(move |path, file| Ok(defaults(path, file)?.1.defaults.clone())),
            resources: Some(resolve),
        }
    }
}

pub(super) fn install(interp: &mut Interp, id: ContextID) -> MoltResult {
    if interp.context::<State>(id).archive_sites.is_some() {
        interp.add_context_command("portarchivefetch::get_sites", get_sites, id);
        for command in [
            "get_full_archive_sites_path",
            "get_archive_site_urls",
            "get_archive_site_archivetype",
            "get_archive_site_sigtype",
            "get_archive_site_pubkey",
        ] {
            interp.add_context_command(&format!("portarchivefetch::{command}"), resource, id);
        }
        defaults::register(interp, id, "archive_sites", "[portarchivefetch::get_sites]")?;
    }
    Ok(Value::empty())
}

fn get_sites(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| {
        let path = interp.scalar("archive_sites.listpath")?.to_string();
        let file = interp.scalar("archive_sites.listfile")?.to_string();
        let resolver = interp
            .context::<State>(id)
            .archive_sites
            .clone()
            .expect("registered archive-site resolver");
        (resolver.defaults)(&path, &file)
            .map(list)
            .map_err(|err| error(format!("archive-site defaults: {err:#}")))
    })
}

fn resource(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    let path_only = argv[0].as_str().ends_with("::get_full_archive_sites_path");
    if path_only {
        check_args(1, argv, 1, 1, "")?;
    } else {
        check_args(1, argv, 3, 3, "mirrorfile site")?;
    }
    interp.with_global_scope(|interp| {
        let path = interp.scalar("archive_sites.listpath")?;
        let file = interp.scalar("archive_sites.listfile")?;
        let resolver = interp
            .context::<State>(id)
            .archive_sites
            .as_ref()
            .and_then(|sites| sites.resources.clone());
        let Some(resolver) = resolver else {
            return commands::unsupported(interp, id, argv);
        };
        let (resource, selection) = resolver(path.as_str(), file.as_str())
            .map_err(|err| error(format!("archive resource: {err:#}")))?;
        if path_only {
            return Ok(resource.into());
        }
        if argv[1].as_str() != resource {
            return Err(error(
                "archive resource path does not match the active selection",
            ));
        }
        let Some(site) = selection.get(argv[2].as_str()) else {
            return Ok(Value::empty());
        };
        match argv[0].as_str().rsplit("::").next().unwrap() {
            "get_archive_site_urls" => Ok(list(site.urls.clone())),
            "get_archive_site_archivetype" => Ok(site.archive_type.clone().into()),
            "get_archive_site_sigtype" => Ok(site.signature_type.clone().into()),
            "get_archive_site_pubkey" => Ok(site.public_key.clone().unwrap_or_default().into()),
            _ => unreachable!(),
        }
    })
}
