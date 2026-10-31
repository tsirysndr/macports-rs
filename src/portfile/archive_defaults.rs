//! Caller-provided archive metadata, resolved lazily during Portfile evaluation.
use super::*;
use molt::{MoltResult, check_args};
use std::rc::Rc;

type Resolver = dyn Fn(&str, &str) -> Result<Vec<String>>;

/// Resolves archive-site defaults for the live listpath/listfile values. The
/// caller captures the active tree, installation layout, and startup config.
#[derive(Clone)]
pub struct ArchiveSites(Rc<Resolver>);
impl ArchiveSites {
    pub fn new(resolve: impl Fn(&str, &str) -> Result<Vec<String>> + 'static) -> Self {
        Self(Rc::new(resolve))
    }
}

pub(super) fn install(interp: &mut Interp, id: ContextID) -> MoltResult {
    if interp.context::<State>(id).archive_sites.is_some() {
        interp.add_context_command("portarchivefetch::get_sites", get_sites, id);
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
        (resolver.0)(&path, &file)
            .map(list)
            .map_err(|err| error(format!("archive-site defaults: {err:#}")))
    })
}
