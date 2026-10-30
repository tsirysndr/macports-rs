//! Reverse registry dependencies, including both active and inactive images.
use super::{Entry, Registry};
use anyhow::Result;
use std::collections::HashMap;

pub struct Graph<'a> {
    entries: &'a [Entry],
    edges: HashMap<String, Vec<usize>>,
}
impl<'a> Graph<'a> {
    pub fn build(registry: &Registry, entries: &'a [Entry]) -> Result<Self> {
        let positions: HashMap<_, _> = entries
            .iter()
            .enumerate()
            .map(|(i, entry)| (entry.id, i))
            .collect();
        let mut edges: HashMap<String, Vec<usize>> = HashMap::new();
        for (name, id) in registry.dependent_edges()? {
            if let Some(&position) = positions.get(&id) {
                edges
                    .entry(name.to_ascii_lowercase())
                    .or_default()
                    .push(position);
            }
        }
        for dependents in edges.values_mut() {
            dependents.sort_unstable();
        }
        Ok(Self { entries, edges })
    }
    pub fn direct(&self, name: &str) -> impl Iterator<Item = &Entry> {
        self.edges
            .get(&name.to_ascii_lowercase())
            .into_iter()
            .flatten()
            .map(|&i| &self.entries[i])
    }
    pub fn walk(
        &self,
        name: &str,
        full: bool,
        mut visit: impl FnMut(&Entry, usize, bool) -> Result<()>,
    ) -> Result<()> {
        enum Event {
            Enter(usize, usize),
            Leave(String),
        }
        let mut stack = Vec::new();
        if let Some(roots) = self.edges.get(&name.to_ascii_lowercase()) {
            stack.extend(roots.iter().rev().map(|&i| Event::Enter(i, 1)));
        }
        // true denotes the current path; false denotes a completed subtree.
        let mut seen = HashMap::new();
        while let Some(event) = stack.pop() {
            match event {
                Event::Leave(key) => {
                    seen.insert(key, false);
                }
                Event::Enter(i, depth) => {
                    let entry = &self.entries[i];
                    let key = entry.name.to_ascii_lowercase();
                    let previous = seen.get(&key).copied();
                    if previous.is_some() && !full {
                        continue;
                    }
                    let cyclic = previous == Some(true);
                    visit(entry, depth, cyclic)?;
                    if cyclic {
                        continue;
                    }
                    seen.insert(key.clone(), true);
                    stack.push(Event::Leave(key.clone()));
                    if let Some(children) = self.edges.get(&key) {
                        stack.extend(children.iter().rev().map(|&i| Event::Enter(i, depth + 1)));
                    }
                }
            }
        }
        Ok(())
    }
}
