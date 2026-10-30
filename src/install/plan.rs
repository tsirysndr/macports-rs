use super::variants;
use crate::{
    dependencies,
    index::{Index, Port},
    query,
    registry::Entry,
    spec::PortSpec,
};
use anyhow::{Context, Result, ensure};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Install,
    Activate,
    AlreadyActive,
}
pub struct Item<'a> {
    pub port: &'a Port,
    pub variants: String,
    pub requested: bool,
    pub action: Action,
}

/// Validate the full reachable graph, then return a dependency-first plan.
/// Iterative traversal keeps deep graphs safe and reports cycles before output.
pub fn plan<'a>(
    index: &'a Index,
    specs: &[PortSpec],
    installed: &[Entry],
) -> Result<Vec<Item<'a>>> {
    let mut images: HashMap<String, Vec<&Entry>> = HashMap::new();
    for entry in installed {
        images
            .entry(entry.name.to_ascii_lowercase())
            .or_default()
            .push(entry);
    }
    let mut requested = HashMap::new();
    for spec in specs {
        let port = query::lookup(index, &spec.name)?;
        if let Some(version) = &spec.version {
            ensure!(
                version == port.get("version") || version == &port.version(),
                "{}: requested version {version} is not the indexed version {}; historical installation requires a matching index",
                port.name,
                port.version()
            );
        }
        let selected = variants::defaults(port, Some(spec))
            .with_context(|| format!("{}: installation variants", port.name))?;
        requested.insert(port.name.clone(), selected);
    }
    enum Event<'a> {
        Enter(&'a Port),
        Leave(Item<'a>),
    }
    let mut stack = Vec::new();
    for spec in specs.iter().rev() {
        stack.push(Event::Enter(query::lookup(index, &spec.name)?));
    }
    let mut visiting = HashSet::new();
    let mut complete = HashSet::new();
    let mut result = Vec::new();
    while let Some(event) = stack.pop() {
        match event {
            Event::Leave(item) => {
                visiting.remove(&item.port.name);
                complete.insert(item.port.name.clone());
                result.push(item);
            }
            Event::Enter(port) => {
                if complete.contains(&port.name) {
                    continue;
                }
                ensure!(
                    visiting.insert(port.name.clone()),
                    "dependency cycle while planning installation of {}",
                    port.name
                );
                ensure!(
                    !port.get("version").is_empty(),
                    "{}: index has no version",
                    port.name
                );
                let revision: i64 = if port.get("revision").is_empty() {
                    0
                } else {
                    port.get("revision")
                        .parse()
                        .context("invalid indexed revision")?
                };
                ensure!(revision >= 0, "{}: negative indexed revision", port.name);
                ensure!(
                    port.get("replaced_by").is_empty(),
                    "{} is replaced by {}; replacement handling is not implemented for install",
                    port.name,
                    port.get("replaced_by")
                );
                let variants = requested
                    .get(&port.name)
                    .cloned()
                    .map(Ok)
                    .unwrap_or_else(|| variants::defaults(port, None))
                    .with_context(|| format!("{}: installation variants", port.name))?;
                let epoch = if port.get("epoch").is_empty() {
                    0
                } else {
                    port.get("epoch")
                        .parse::<i64>()
                        .context("invalid indexed epoch")?
                };
                let candidates = images
                    .get(&port.name.to_ascii_lowercase())
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                for entry in candidates {
                    if !entry.epoch.is_empty() {
                        entry
                            .epoch
                            .parse::<i64>()
                            .context("invalid installed epoch")?;
                    }
                }
                let matches = |entry: &&Entry| {
                    entry.name.eq_ignore_ascii_case(&port.name)
                        && entry.version == port.get("version")
                        && entry.revision == revision
                        && entry.variants == variants
                        && entry.epoch.parse::<i64>().unwrap_or(0) == epoch
                };
                let active = candidates.iter().copied().find(|entry| entry.active);
                let action = if let Some(active) = active {
                    ensure!(
                        matches(&active),
                        "{} @{} is already active with a different version, epoch, or variants; upgrades and replacement activation are not implemented",
                        active.name,
                        active.full_version()
                    );
                    Action::AlreadyActive
                } else if candidates.iter().any(matches) {
                    Action::Activate
                } else {
                    Action::Install
                };
                let item = Item {
                    port,
                    variants,
                    requested: requested.contains_key(&port.name),
                    action,
                };
                // Active images already have a registered dependency graph; don't
                // pretend their indexed dependencies need reinstalling.
                if action == Action::AlreadyActive {
                    visiting.remove(&port.name);
                    complete.insert(port.name.clone());
                    result.push(item);
                    continue;
                }
                let mut targets = Vec::new();
                let mut seen = HashSet::new();
                for field in ["depends_lib", "depends_run"] {
                    for dependency in dependencies::in_field(port, field)? {
                        ensure!(
                            dependency
                                .specification
                                .strip_prefix("port:")
                                .is_some_and(|name| !name.contains(':') && !name.contains('/')),
                            "{}: provider dependency {} requires provider resolution, which is not implemented for install",
                            port.name,
                            dependency.specification
                        );
                        let target = query::lookup(index, &dependency.name).with_context(|| {
                            format!("{}: {field} {}", port.name, dependency.specification)
                        })?;
                        if seen.insert(target.name.clone()) {
                            targets.push(target);
                        }
                    }
                }
                stack.push(Event::Leave(item));
                stack.extend(targets.into_iter().rev().map(Event::Enter));
            }
        }
    }
    Ok(result)
}
