//! Read-only registry aliases using fresh snapshots after dependency installs.
use super::*;
use molt::{MoltResult, check_args};
use std::path::{Path, PathBuf};
pub(super) enum Source {
    Missing,
    Database(PathBuf),
}
impl Source {
    pub(super) fn from_path(path: Option<&Path>) -> Self {
        match path {
            Some(path) => Self::Database(path.to_owned()),
            None => Self::Missing,
        }
    }
}
impl Session {
    /// Bind the startup driver's selected registry. None explicitly denotes an
    /// absent registry; an unbound session cannot guess or use the host default.
    /// Each lookup opens a fresh read-only snapshot, seeing committed installs.
    pub fn set_registry_lookup(&mut self, path: Option<&Path>) {
        self.interp.context::<State>(self.id).registry_lookup = Some(match path {
            Some(path) => Source::Database(path.to_owned()),
            None => Source::Missing,
        });
    }
}
pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("registry_exists", exists, id);
    interp.add_context_command("registry_exists_for_name", exists, id);
    interp.add_context_command("registry_file_registered", file_registered, id);
    interp.add_context_command("_get_dep_port", dep_port, id);
    interp.add_context_command("registry_active", active, id);
    interp.add_context_command("registry_deactivate_composite", deactivate_composite, id);
}
/// Deactivate an installed port from a Portfile hook (upstream
/// `portimage::deactivate_composite`), e.g. one a new port replaces.
fn deactivate_composite(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 4, "name ?version? ?options?")?;
    let name = argv[1].as_str();
    if name.is_empty() {
        return Err(error(
            "Registry error: Please specify the name of the port.",
        ));
    }
    let wanted = argv.get(2).map(Value::as_str).unwrap_or("");
    let options = argv
        .get(3)
        .map(Value::as_list)
        .transpose()?
        .unwrap_or_default();
    let option = |key: &str| {
        options.chunks(2).any(|pair| {
            pair[0].as_str() == key
                && pair
                    .get(1)
                    .is_some_and(|value| value.as_bool().unwrap_or(false))
        })
    };
    let (force, nodepcheck) = (option("ports_force"), option("ports_nodepcheck"));
    let Some(Source::Database(path)) = &interp.context::<State>(id).registry_lookup else {
        return Err(error(format!("Image error: port {name} is not active.")));
    };
    let path = path.clone();
    let progress = interp
        .context::<State>(id)
        .progress
        .clone()
        .unwrap_or_else(|| crate::progress::Progress::new(false));
    let registry = crate::registry::Registry::open(Some(&path))
        .map_err(|err| error(err.to_string()))?
        .ok_or_else(|| error(format!("Image error: port {name} is not active.")))?;
    let entries = registry.entries().map_err(|err| error(err.to_string()))?;
    let matches = |entry: &&crate::registry::Entry| {
        entry.name.eq_ignore_ascii_case(name)
            && (wanted.is_empty() || wanted == entry.version || wanted == entry.full_version())
    };
    let Some(active) = entries
        .iter()
        .find(|entry| entry.active && entry.name.eq_ignore_ascii_case(name))
    else {
        if entries.iter().any(|entry| matches(&entry)) {
            progress.log("info", &format!("port {name} is already inactive"));
            return Ok(Value::empty());
        }
        let version = if wanted.is_empty() {
            String::new()
        } else {
            format!(" @{wanted}")
        };
        return Err(error(format!(
            "Image error: port {name}{version} is not active."
        )));
    };
    let specifier = active.full_version();
    if !matches(&active) {
        if entries.iter().any(|entry| !entry.active && matches(&entry)) {
            progress.log(
                "info",
                &format!("port {name} @{wanted} is already inactive"),
            );
            return Ok(Value::empty());
        }
        return Err(error(format!(
            "Active version of {} is not {wanted} but {specifier}.",
            active.name
        )));
    }
    if !nodepcheck {
        let ids: Vec<i64> = registry
            .dependent_edges()
            .map_err(|err| error(err.to_string()))?
            .into_iter()
            .filter(|(dependency, _)| dependency.eq_ignore_ascii_case(&active.name))
            .map(|(_, dependent)| dependent)
            .collect();
        let mut blocking: Vec<&str> = entries
            .iter()
            .filter(|entry| entry.active && ids.contains(&entry.id))
            .map(|entry| entry.name.as_str())
            .collect();
        blocking.sort_unstable();
        blocking.dedup();
        if !blocking.is_empty() {
            let list = blocking
                .iter()
                .map(|name| format!("  {name}"))
                .collect::<Vec<_>>()
                .join("\n");
            if !force {
                return Err(error(format!(
                    "Unable to deactivate {} @{specifier}, the following ports depend on it:\n{list}",
                    active.name
                )));
            }
            progress.warning(&format!(
                "{} @{specifier} is needed by the following ports, but deactivate was forced:\n{list}",
                active.name
            ));
        }
    }
    let files = registry
        .files(active)
        .map_err(|err| error(err.to_string()))?;
    drop(registry);
    progress.phase(&format!("Deactivating {} @{specifier}", active.name));
    let mut writer =
        crate::registry::write::Writer::open(&path).map_err(|err| error(err.to_string()))?;
    crate::install::deactivation::deactivate(&mut writer, active.id, &files)
        .map_err(|err| error(format!("{err:#}")))?;
    Ok(Value::empty())
}
/// Active images as `{name version revision variants active epoch}` lists
/// (upstream `registry::active`), failing when none match.
fn active(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 2, "?name?")?;
    let name = argv.get(1).map(Value::as_str).unwrap_or("");
    let entries = match &interp.context::<State>(id).registry_lookup {
        Some(Source::Database(path)) => crate::registry::Registry::open(Some(path))
            .and_then(|registry| registry.map(|registry| registry.entries()).transpose())
            .map_err(|err| error(err.to_string()))?
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let active: Vec<Value> = entries
        .iter()
        .filter(|entry| entry.active && (name.is_empty() || entry.name.eq_ignore_ascii_case(name)))
        .map(|entry| {
            Value::from(vec![
                Value::from(entry.name.as_str()),
                Value::from(entry.version.as_str()),
                Value::from(entry.revision),
                Value::from(entry.variants.as_str()),
                Value::from(1),
                Value::from(entry.epoch.as_str()),
            ])
        })
        .collect();
    if active.is_empty() {
        return Err(error(if name.is_empty() {
            "Registry error: No ports registered as active.".to_owned()
        } else {
            format!("Registry error: {name} not registered as installed & active.")
        }));
    }
    Ok(Value::from(active))
}
/// The port satisfying a depspec, or empty for an unregistered system
/// provider (upstream portutil `_get_dep_port`).
fn dep_port(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "depspec")?;
    let registry = match &interp.context::<State>(id).registry_lookup {
        Some(Source::Database(path)) => {
            crate::registry::Registry::open(Some(path)).map_err(|err| error(err.to_string()))?
        }
        _ => None,
    };
    let depspec = argv[1].as_str();
    let name = depspec.rsplit(':').next().unwrap_or(depspec);
    // An active port of the fallback name wins before any filesystem probe.
    if let Some(registry) = &registry
        && registry
            .entries()
            .map_err(|err| error(err.to_string()))?
            .iter()
            .any(|entry| entry.active && entry.name.eq_ignore_ascii_case(name))
    {
        return Ok(name.into());
    }
    let search = interp.with_global_scope(|interp| {
        let mut environment = std::collections::BTreeMap::new();
        for name in [
            "PATH",
            "DYLD_FRAMEWORK_PATH",
            "DYLD_FALLBACK_FRAMEWORK_PATH",
            "DYLD_LIBRARY_PATH",
            "DYLD_FALLBACK_LIBRARY_PATH",
        ] {
            if let Ok(value) = interp.element("env", name) {
                environment.insert(name.to_owned(), value.to_string());
            }
        }
        Ok(crate::dependencies::provider::Search {
            prefix: interp.scalar("prefix")?.as_str().into(),
            frameworks_dir: interp.scalar("frameworks_dir")?.as_str().into(),
            platform: interp.scalar("os.platform")?.to_string(),
            environment,
        })
    })?;
    crate::dependencies::provider::Resolver::new(registry.as_ref(), search)
        .and_then(|resolver| resolver.resolve(depspec))
        .map(|name| Value::from(name.unwrap_or_default()))
        .map_err(|err| error(err.to_string()))
}
/// The active port that installed `path`, or 0 (upstream `registry::file_registered`).
fn file_registered(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "path")?;
    let path = match &interp.context::<State>(id).registry_lookup {
        Some(Source::Database(path)) => path.clone(),
        _ => return Ok(0.into()),
    };
    let owner = crate::registry::Registry::open(Some(&path))
        .ok()
        .flatten()
        .and_then(|registry| registry.owner(argv[1].as_str(), true).ok().flatten());
    Ok(owner.map_or_else(|| 0.into(), Value::from))
}
fn exists(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    let name_only = argv[0].as_str() == "registry_exists_for_name";
    if name_only {
        check_args(1, argv, 2, 2, "name")?;
    } else {
        check_args(1, argv, 3, 5, "name version ?revision? ?variants?")?;
    }
    let source = &interp.context::<State>(id).registry_lookup;
    let path = match source {
        None => return commands::unsupported(interp, id, argv),
        Some(Source::Missing) => return Ok(false.into()),
        Some(Source::Database(path)) => path.clone(),
    };
    // receipt_sqlite::entry_exists catches registry search errors and returns 0.
    let found = (|| -> Result<bool> {
        let Some(registry) = crate::registry::Registry::open(Some(&path))? else {
            return Ok(false);
        };
        if name_only {
            registry.contains_name(argv[1].as_str())
        } else {
            registry.contains_image(
                argv[1].as_str(),
                argv[2].as_str(),
                argv.get(3).map(Value::as_str).unwrap_or("0"),
                argv.get(4).map(Value::as_str).unwrap_or(""),
            )
        }
    })()
    .unwrap_or(false);
    Ok(found.into())
}
