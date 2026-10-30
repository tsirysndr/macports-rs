use crate::{fields, index::Index, tcl_list};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};

/// Indexed reverse edges, built once and shared by selectors in an expression.
pub struct ReverseIndex {
    edges: HashMap<String, Vec<String>>,
}

impl ReverseIndex {
    pub fn build(index: &Index) -> Result<Self> {
        let mut edges: HashMap<String, Vec<String>> = HashMap::new();
        for port in index.ports().values() {
            let mut seen = HashSet::new();
            for field in fields::DEPENDENCIES {
                for specification in tcl_list::parse(port.get(field))
                    .with_context(|| format!("{}: {field}", port.name))?
                {
                    // Upstream indexes the final component, even for unusual provider syntax.
                    let key = specification
                        .rsplit(':')
                        .next()
                        .unwrap_or("")
                        .to_lowercase();
                    if seen.insert(key.clone()) {
                        edges.entry(key).or_default().push(port.name.clone());
                    }
                }
            }
        }
        Ok(Self { edges })
    }

    /// Exact, case-insensitive names, including references absent from the index.
    /// A root is included only if reached through a dependency cycle.
    pub fn recursive(&self, name: &str) -> Vec<String> {
        let mut pending = vec![name.to_lowercase()];
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        while let Some(name) = pending.pop() {
            if let Some(dependents) = self.edges.get(&name) {
                for dependent in dependents {
                    let key = dependent.to_lowercase();
                    if seen.insert(key.clone()) {
                        result.push(dependent.clone());
                        pending.push(key);
                    }
                }
            }
        }
        result.sort_by(|a, b| crate::order::dictionary(a, b));
        result
    }
}
