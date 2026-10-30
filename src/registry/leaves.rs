//! Linear-time leaf selection, including inactive images and dependency cycles.
use super::{Entry, Registry};
use anyhow::Result;
use std::collections::{HashMap, HashSet};

pub(super) struct Leaves {
    depended_on: HashSet<String>,
    needed: HashSet<String>,
}
impl Leaves {
    pub fn read(registry: Option<&Registry>, entries: &[Entry]) -> Result<Self> {
        let mut dependencies: HashMap<String, Vec<String>> = HashMap::new();
        let mut depended_on = HashSet::new();
        let owners: HashMap<_, _> = entries.iter().map(|entry| (entry.id, entry)).collect();
        if let Some(registry) = registry {
            for (target, id) in registry.dependent_edges()? {
                if let Some(owner) = owners.get(&id) {
                    let target = target.to_ascii_lowercase();
                    depended_on.insert(target.clone());
                    dependencies
                        .entry(owner.name.to_ascii_lowercase())
                        .or_default()
                        .push(target);
                }
            }
        }
        let mut pending: Vec<_> = entries
            .iter()
            .filter(|entry| entry.requested)
            .map(|entry| entry.name.to_ascii_lowercase())
            .collect();
        let mut visited = HashSet::new();
        let mut needed = HashSet::new();
        while let Some(name) = pending.pop() {
            if !visited.insert(name.clone()) {
                continue;
            }
            if let Some(targets) = dependencies.get(&name) {
                for target in targets {
                    needed.insert(target.clone());
                    pending.push(target.clone());
                }
            }
        }
        Ok(Self {
            depended_on,
            needed,
        })
    }
    pub fn matches(&self, entry: &Entry, recursive: bool) -> bool {
        !entry.requested
            && !if recursive {
                &self.needed
            } else {
                &self.depended_on
            }
            .contains(&entry.name.to_ascii_lowercase())
    }
}
