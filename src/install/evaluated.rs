//! Owned Portfile metadata for preliminary binary installation previews.
use super::{Action, Item, plan, variants::Selection};
use crate::{
    dependencies,
    index::{Index, Port},
    portfile::Evaluation,
    query,
    registry::Entry,
    spec::PortSpec,
};
use anyhow::{Context, Result, ensure};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

pub struct EvaluatedPlan {
    evaluations: BTreeMap<String, Evaluation>,
    steps: Vec<Step>,
}
struct Step {
    name: String,
    variants: String,
    provider_fallbacks: Vec<String>,
    requested: bool,
    action: Action,
}
impl EvaluatedPlan {
    /// Includes deferred hooks and target records. A successful preview is not
    /// authorization to skip PortSystem initialization, hooks, or image checks.
    pub fn evaluations(&self) -> &BTreeMap<String, Evaluation> {
        &self.evaluations
    }
    pub fn items(&self) -> impl Iterator<Item = Item<'_>> {
        self.steps.iter().map(|step| Item {
            port: &self.evaluations[&step.name].port,
            variants: step.variants.clone(),
            variants_resolved: true,
            provider_fallbacks: step.provider_fallbacks.clone(),
            requested: step.requested,
            action: step.action,
        })
    }
}

/// Evaluate the runtime dependency closure, then apply the same image/cycle
/// checks as indexed previews. The loader must evaluate the supplied spec.
/// Explicitly requested roots take precedence over inherited dependency inputs.
/// Different contexts for a shared dependency must produce identical metadata.
pub fn plan_evaluated(
    index: &Index,
    specs: &[PortSpec],
    installed: &[Entry],
    load: impl FnMut(&Port, &PortSpec) -> Result<Evaluation>,
) -> Result<EvaluatedPlan> {
    plan_inner(index, specs, installed, |_, _| Ok(None), load)
}

/// Evaluate only the dependency closure not already satisfied by provider
/// evidence. Explicit roots still take precedence. The resolver and installed
/// entries must describe the same registry snapshot.
pub fn plan_evaluated_with_providers(
    index: &Index,
    specs: &[PortSpec],
    installed: &[Entry],
    resolver: &dependencies::provider::Resolver<'_>,
    load: impl FnMut(&Port, &PortSpec) -> Result<Evaluation>,
) -> Result<EvaluatedPlan> {
    let mut decisions = HashMap::new();
    plan_inner(
        index,
        specs,
        installed,
        |_, dependency| {
            if let Some(value) = decisions.get(dependency) {
                return Ok(Some(*value));
            }
            let value = resolver.probe(dependency)?.is_satisfied();
            decisions.insert(dependency.to_owned(), value);
            Ok(Some(value))
        },
        load,
    )
}

/// Dependency order and retained interpreters for subsequent target execution.
pub struct SessionPlan {
    pub plan: EvaluatedPlan,
    pub sessions: BTreeMap<String, crate::portfile::Session>,
}

/// Plan using each loaded session's provider search context. The caller must
/// initialize the context before returning the session; target hooks are not run.
/// Decisions are cached per dependent port and reused for final graph ordering.
pub fn plan_sessions(
    index: &Index,
    specs: &[PortSpec],
    registry: Option<&crate::registry::Registry>,
    mut load: impl FnMut(&Port, &PortSpec) -> Result<crate::portfile::Session>,
) -> Result<SessionPlan> {
    use dependencies::provider::{Resolver, Search};
    use std::cell::RefCell;
    let installed = registry
        .map(crate::registry::Registry::entries)
        .transpose()?
        .unwrap_or_default();
    let resolvers = RefCell::new(HashMap::new());
    let mut contexts = HashMap::new();
    let mut sessions = BTreeMap::new();
    let plan = plan_inner(
        index,
        specs,
        &installed,
        |port, dependency| {
            let resolvers = resolvers.borrow();
            let resolver: &Resolver<'_> = resolvers
                .get(&port.name)
                .context("missing session provider context")?;
            Ok(Some(resolver.probe(dependency)?.is_satisfied()))
        },
        |port, spec| {
            let mut session = load(port, spec)?;
            let search = Search::from_session(&mut session)?;
            let mut resolvers = resolvers.borrow_mut();
            // Different inherited inputs may evaluate a shared port repeatedly.
            // Metadata equality alone cannot prove identical provider context.
            if let Some(previous) = contexts.insert(port.name.clone(), search.clone()) {
                ensure!(
                    previous == search,
                    "{}: conflicting provider search contexts from multiple dependency paths",
                    port.name
                );
            }
            let resolver = match resolvers.values().next() {
                Some(previous) => previous.with_search(search),
                None => Resolver::from_entries(registry, search, &installed),
            };
            resolvers.insert(port.name.clone(), resolver);
            let metadata = session.metadata().clone();
            sessions.entry(port.name.clone()).or_insert(session);
            Ok(metadata)
        },
    )?;
    Ok(SessionPlan { plan, sessions })
}

fn plan_inner(
    index: &Index,
    specs: &[PortSpec],
    installed: &[Entry],
    mut probe: impl FnMut(&Port, &str) -> Result<Option<bool>>,
    mut load: impl FnMut(&Port, &PortSpec) -> Result<Evaluation>,
) -> Result<EvaluatedPlan> {
    let mut images: HashMap<String, Vec<&Entry>> = HashMap::new();
    for entry in installed {
        images
            .entry(entry.name.to_ascii_lowercase())
            .or_default()
            .push(entry);
    }
    let mut roots = HashMap::new();
    let mut pending = VecDeque::new();
    for spec in specs {
        let mut spec = spec.clone();
        spec.name = query::lookup(index, &spec.name)?.name.clone();
        if let Some(previous) = roots.insert(spec.name.clone(), spec.clone()) {
            ensure!(
                previous == spec,
                "{}: conflicting requested build inputs",
                spec.name
            );
        }
        pending.push_back((spec, None::<String>));
    }
    let mut evaluated: BTreeMap<String, Evaluation> = BTreeMap::new();
    let explicit: HashSet<_> = roots.keys().map(|name| name.to_ascii_lowercase()).collect();
    let mut providers = HashMap::new();
    let mut contexts: HashMap<String, Vec<PortSpec>> = HashMap::new();
    while let Some((spec, parent)) = pending.pop_front() {
        let seen = contexts.entry(spec.name.clone()).or_default();
        if seen.contains(&spec) {
            continue;
        }
        seen.push(spec.clone());
        let indexed = query::lookup(index, &spec.name)?;
        let value = load(indexed, &spec).with_context(|| {
            parent.unwrap_or_else(|| format!("{}: install metadata", spec.name))
        })?;
        ensure!(
            value.port.name == spec.name,
            "Portfile returned a different port name"
        );
        if let Some(previous) = evaluated.get(&spec.name) {
            ensure!(
                previous == &value,
                "{}: conflicting evaluated build inputs from multiple dependency paths",
                spec.name
            );
        } else {
            evaluated.insert(spec.name.clone(), value);
        }
        let value = &evaluated[&spec.name];
        if matching_active(
            value,
            images
                .get(&spec.name.to_ascii_lowercase())
                .map(Vec::as_slice)
                .unwrap_or_default(),
        ) {
            continue;
        }
        for field in ["depends_lib", "depends_run"] {
            for dependency in dependencies::in_field(&value.port, field)? {
                let context = format!("{}: {field} {}", spec.name, dependency.specification);
                let key = (value.port.name.clone(), dependency.specification.clone());
                let satisfied = match providers.get(&key) {
                    Some(value) => *value,
                    None => {
                        let result = probe(&value.port, &dependency.specification)
                            .with_context(|| context.clone())?;
                        providers.insert(key, result);
                        result
                    }
                };
                if satisfied == Some(true)
                    && !explicit.contains(&dependency.name.to_ascii_lowercase())
                {
                    continue;
                }
                let target =
                    query::lookup(index, &dependency.name).with_context(|| context.clone())?;
                let inherited = roots.get(&target.name).cloned().unwrap_or_else(|| {
                    let mut inherited = spec.clone();
                    inherited.name = target.name.clone();
                    inherited.version = None;
                    inherited
                });
                pending.push_back((inherited, Some(context)));
            }
        }
    }
    let items = plan::plan_providers(
        |name| {
            let canonical = &query::lookup(index, name)?.name;
            evaluated
                .get(canonical)
                .map(|value| &value.port)
                .with_context(|| format!("no evaluated metadata for {name}"))
        },
        |port, _| {
            Ok(Selection {
                enabled: evaluated[&port.name].enabled.clone(),
                resolved: true,
            })
        },
        specs,
        installed,
        |port, dependency| {
            providers
                .get(&(port.name.clone(), dependency.to_owned()))
                .copied()
                .context("missing evaluated provider decision")
        },
    )?;
    let steps = items
        .into_iter()
        .map(|item| Step {
            name: item.port.name.clone(),
            variants: item.variants,
            provider_fallbacks: item.provider_fallbacks,
            requested: item.requested,
            action: item.action,
        })
        .collect();
    Ok(EvaluatedPlan {
        evaluations: evaluated,
        steps,
    })
}

fn matching_active(value: &Evaluation, installed: &[&Entry]) -> bool {
    let port = &value.port;
    let revision = port.get("revision").parse::<i64>().ok();
    let epoch = port.get("epoch").parse::<i64>().ok();
    installed.iter().any(|entry| {
        entry.active
            && entry.name.eq_ignore_ascii_case(&port.name)
            && entry.version == port.get("version")
            && Some(entry.revision) == revision
            && Some(entry.epoch.parse::<i64>().unwrap_or(0)) == epoch
            && entry.variants == value.enabled
    })
}
