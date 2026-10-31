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
    Resolve,
}
pub struct Item<'a> {
    pub port: &'a Port,
    pub variants: String,
    pub variants_resolved: bool,
    pub provider_fallbacks: Vec<String>,
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
    plan_with(
        |name| query::lookup(index, name),
        variants::defaults,
        specs,
        installed,
    )
}

pub(super) fn plan_with<'a>(
    lookup: impl Fn(&str) -> Result<&'a Port>,
    select: impl Fn(&Port, Option<&PortSpec>) -> Result<variants::Selection>,
    specs: &[PortSpec],
    installed: &[Entry],
) -> Result<Vec<Item<'a>>> {
    plan_providers(lookup, select, specs, installed, RUNTIME, |_, _| Ok(None))
}

/// Plan using current provider evidence. Use entries and a resolver from the same
/// registry snapshot. This still requires evaluated/default variant metadata;
/// it does not initialize PortSystem or execute hooks.
pub fn plan_with_providers<'a>(
    index: &'a Index,
    specs: &[PortSpec],
    installed: &[Entry],
    resolver: &dependencies::provider::Resolver<'_>,
) -> Result<Vec<Item<'a>>> {
    plan_providers(
        |name| query::lookup(index, name),
        variants::defaults,
        specs,
        installed,
        RUNTIME,
        |_, dependency| Ok(Some(resolver.probe(dependency)?.is_satisfied())),
    )
}

/// Dependency fields a binary installation needs active first.
pub(super) const RUNTIME: &[&str] = &["depends_lib", "depends_run"];

pub(super) fn plan_providers<'a>(
    lookup: impl Fn(&str) -> Result<&'a Port>,
    select: impl Fn(&Port, Option<&PortSpec>) -> Result<variants::Selection>,
    specs: &[PortSpec],
    installed: &[Entry],
    fields: &[&str],
    mut provider: impl FnMut(&Port, &str) -> Result<Option<bool>>,
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
        let port = lookup(&spec.name)?;
        if let Some(version) = &spec.version {
            ensure!(
                version == port.get("version") || version == &port.version(),
                "{}: requested version {version} does not match available metadata version {}; historical installation requires matching metadata",
                port.name,
                port.version()
            );
        }
        let selected = select(port, Some(spec))
            .with_context(|| format!("{}: installation variants", port.name))?;
        requested.insert(port.name.clone(), selected);
    }
    enum Event<'a> {
        Enter(&'a Port),
        Leave(Item<'a>),
    }
    let explicit: HashSet<_> = requested
        .keys()
        .map(|name| name.to_ascii_lowercase())
        .collect();
    let mut stack = Vec::new();
    for spec in specs.iter().rev() {
        stack.push(Event::Enter(lookup(&spec.name)?));
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
                    "{}: port metadata has no version",
                    port.name
                );
                let revision: i64 = if port.get("revision").is_empty() {
                    0
                } else {
                    port.get("revision")
                        .parse()
                        .context("invalid port revision")?
                };
                ensure!(revision >= 0, "{}: negative port revision", port.name);
                ensure!(
                    port.get("replaced_by").is_empty(),
                    "{} is replaced by {}; replacement handling is not implemented for install",
                    port.name,
                    port.get("replaced_by")
                );
                let selection = requested
                    .get(&port.name)
                    .cloned()
                    .map(Ok)
                    .unwrap_or_else(|| select(port, None))
                    .with_context(|| format!("{}: installation variants", port.name))?;
                let variants = selection.enabled;
                let epoch = if port.get("epoch").is_empty() {
                    0
                } else {
                    port.get("epoch")
                        .parse::<i64>()
                        .context("invalid port epoch")?
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
                let action = if !selection.resolved {
                    Action::Resolve
                } else if active.is_some_and(|active| matches(&active))
                    // An active dependency of any version satisfies a port:
                    // dependency; only requested ports are replaced.
                    || (active.is_some() && !requested.contains_key(&port.name))
                {
                    Action::AlreadyActive
                } else if candidates.iter().any(matches) {
                    // Includes replacing a different active version, which
                    // activation deactivates first, as upstream does.
                    Action::Activate
                } else {
                    Action::Install
                };
                let mut item = Item {
                    port,
                    variants,
                    variants_resolved: selection.resolved,
                    provider_fallbacks: Vec::new(),
                    requested: requested.contains_key(&port.name),
                    action,
                };
                // Active images already have a registered dependency graph; don't
                // pretend their declared dependencies need reinstalling.
                if action == Action::AlreadyActive {
                    visiting.remove(&port.name);
                    complete.insert(port.name.clone());
                    result.push(item);
                    continue;
                }
                let mut targets = Vec::new();
                let mut seen = HashSet::new();
                for field in fields {
                    for dependency in dependencies::in_field(port, field)? {
                        let components: Vec<_> = dependency.specification.split(':').collect();
                        let probe_required = match components.as_slice() {
                            ["port", name] if !name.is_empty() && !name.contains('/') => false,
                            ["bin" | "lib" | "path", probe, name]
                                if !probe.is_empty() && !name.is_empty() =>
                            {
                                true
                            }
                            _ => anyhow::bail!(
                                "{}: unsupported provider dependency {}",
                                port.name,
                                dependency.specification
                            ),
                        };
                        let provided =
                            provider(port, &dependency.specification).with_context(|| {
                                format!("{}: {field} {}", port.name, dependency.specification)
                            })?;
                        if provided == Some(true)
                            && !explicit.contains(&dependency.name.to_ascii_lowercase())
                        {
                            continue;
                        }
                        if probe_required && provided.is_none() {
                            item.provider_fallbacks
                                .push(dependency.specification.clone());
                        }
                        let target = lookup(&dependency.name).with_context(|| {
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
