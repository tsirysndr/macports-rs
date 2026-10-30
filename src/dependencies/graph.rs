use super::{Dependency, Filter, of};
use crate::{
    index::{Index, Port},
    query,
};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet, VecDeque};

/// Reachable dependency metadata is validated before rendering, including missing targets.
pub struct Graph<'a> {
    pub(super) root: &'a Port,
    edges: HashMap<String, Vec<Dependency>>,
}
impl<'a> Graph<'a> {
    pub fn build(index: &'a Index, name: &str, filter: Filter) -> Result<Self> {
        let root = query::lookup(index, name)?;
        let mut edges = HashMap::new();
        let mut pending = VecDeque::from([root]);
        let mut queued = HashSet::from([root.name.to_lowercase()]);
        while let Some(port) = pending.pop_front() {
            let dependencies = of(port, filter)?;
            for dependency in &dependencies {
                if queued.insert(dependency.name.to_lowercase()) {
                    let target = query::lookup(index, &dependency.name).with_context(|| {
                        format!("{} depends on {}", port.name, dependency.specification)
                    })?;
                    pending.push_back(target);
                }
            }
            edges.insert(port.name.to_lowercase(), dependencies);
        }
        Ok(Self { root, edges })
    }

    /// Iterative DFS avoids stack overflow on deep graphs. `full` repeats shared subtrees.
    /// Cycles are suppressed normally and annotated in full-tree output, as in upstream.
    pub fn walk(
        &self,
        full: bool,
        mut visit: impl FnMut(&Dependency, usize, bool) -> Result<()>,
    ) -> Result<()> {
        enum Event<'a> {
            Enter(&'a Dependency, usize),
            Leave(String),
        }
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum State {
            Active,
            Complete,
        }
        let mut state = HashMap::new();
        let mut stack = Vec::new();
        for edge in self.edges[&self.root.name.to_lowercase()].iter().rev() {
            stack.push(Event::Enter(edge, 1));
        }
        while let Some(event) = stack.pop() {
            match event {
                Event::Leave(key) => {
                    state.insert(key, State::Complete);
                }
                Event::Enter(edge, depth) => {
                    let key = edge.name.to_lowercase();
                    let previous = state.get(&key).copied();
                    if previous.is_some() && !full {
                        continue;
                    }
                    let cyclic = previous == Some(State::Active);
                    visit(edge, depth, cyclic)?;
                    if cyclic {
                        continue;
                    }
                    state.insert(key.clone(), State::Active);
                    stack.push(Event::Leave(key.clone()));
                    for child in self.edges[&key].iter().rev() {
                        stack.push(Event::Enter(child, depth + 1));
                    }
                }
            }
        }
        Ok(())
    }
    pub fn is_empty(&self) -> bool {
        self.edges[&self.root.name.to_lowercase()].is_empty()
    }
}
