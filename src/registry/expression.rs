//! Boolean selection over installed image identities, preserving distinct versions.
use super::{Entry, Registry, dependents::Graph, leaves::Leaves, specifications};
use crate::{
    query::Matcher,
    spec::{PortSpec, is_modifier},
};
use anyhow::{Result, bail, ensure};
use std::collections::{BTreeSet, HashSet};

#[derive(Clone, Copy)]
pub enum Missing {
    Error,
    Empty,
}

pub fn select<'a>(
    entries: &'a [Entry],
    tokens: &[String],
    missing: Missing,
    registry: Option<&Registry>,
) -> Result<Vec<&'a Entry>> {
    if tokens.is_empty() {
        return Ok(entries.iter().collect());
    }
    let mut parser = Expression {
        entries,
        tokens,
        cursor: 0,
        depth: 0,
        missing,
        registry,
        leaves: None,
        dependents: None,
        actinact: None,
    };
    let selected = parser.sequence()?;
    ensure!(
        parser.cursor == tokens.len(),
        "unexpected token {:?}",
        parser.peek().unwrap_or("")
    );
    Ok(selected.into_iter().map(|i| &entries[i]).collect())
}
struct Expression<'a> {
    entries: &'a [Entry],
    tokens: &'a [String],
    cursor: usize,
    depth: usize,
    missing: Missing,
    registry: Option<&'a Registry>,
    leaves: Option<Leaves>,
    dependents: Option<Graph<'a>>,
    actinact: Option<HashSet<String>>,
}
impl Expression<'_> {
    fn peek(&self) -> Option<&str> {
        self.tokens.get(self.cursor).map(String::as_str)
    }
    fn sequence(&mut self) -> Result<BTreeSet<usize>> {
        let mut result = self.disjunction()?;
        while self.peek().is_some_and(|token| token != ")") {
            result.extend(self.disjunction()?);
        }
        Ok(result)
    }
    fn disjunction(&mut self) -> Result<BTreeSet<usize>> {
        let mut result = self.conjunction()?;
        while self.peek() == Some("or") {
            self.cursor += 1;
            result.extend(self.conjunction()?);
        }
        Ok(result)
    }
    fn conjunction(&mut self) -> Result<BTreeSet<usize>> {
        let mut result = self.unary()?;
        while self.peek() == Some("and") {
            self.cursor += 1;
            let other = self.unary()?;
            result.retain(|id| other.contains(id));
        }
        Ok(result)
    }
    fn unary(&mut self) -> Result<BTreeSet<usize>> {
        ensure!(
            self.depth < 128,
            "registry expression nesting exceeds 128 levels"
        );
        self.depth += 1;
        let result = match self.peek() {
            Some("not" | "!") => {
                self.cursor += 1;
                let excluded = self.unary()?;
                (0..self.entries.len())
                    .filter(|id| !excluded.contains(id))
                    .collect()
            }
            Some("(") => {
                self.cursor += 1;
                let result = self.sequence()?;
                ensure!(
                    self.peek() == Some(")"),
                    "missing closing parenthesis in registry expression"
                );
                self.cursor += 1;
                result
            }
            Some("and" | "or" | ")" | ";") | None => bail!(
                "expected a registry selector, found {:?}",
                self.peek().unwrap_or("end of expression")
            ),
            Some(_) => self.atom()?,
        };
        self.depth -= 1;
        Ok(result)
    }
    fn atom(&mut self) -> Result<BTreeSet<usize>> {
        let start = self.cursor;
        self.cursor += 1;
        while self.peek().is_some_and(is_modifier) {
            self.cursor += 1;
        }
        if let Some((selector, name)) = self.tokens[start].split_once(':')
            && matches!(selector, "dependentof" | "rdependentof")
        {
            return self.dependency_atom(name, selector == "rdependentof", start + 1);
        }
        let specs = specifications(&self.tokens[start..self.cursor])?;
        let spec = &specs[0];
        let pseudo = matches!(
            spec.name.as_str(),
            "all"
                | "installed"
                | "active"
                | "inactive"
                | "actinact"
                | "requested"
                | "unrequested"
                | "leaves"
                | "rleaves"
        );
        if spec.name == "actinact" && self.actinact.is_none() {
            let active: HashSet<_> = self
                .entries
                .iter()
                .filter(|entry| entry.active)
                .map(|entry| entry.name.to_ascii_lowercase())
                .collect();
            self.actinact = Some(
                self.entries
                    .iter()
                    .filter(|entry| !entry.active)
                    .map(|entry| entry.name.to_ascii_lowercase())
                    .filter(|name| active.contains(name))
                    .collect(),
            );
        }
        if matches!(spec.name.as_str(), "leaves" | "rleaves") && self.leaves.is_none() {
            self.leaves = Some(Leaves::read(self.registry, self.entries)?);
        }
        let glob = spec.name.contains(['*', '?', '[', ']']);
        let matcher = Matcher::new(&spec.name, !glob, false, false)?;
        let version = spec.composite_version();
        let result: BTreeSet<_> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                let name_matches = match spec.name.as_str() {
                    "all" | "installed" => true,
                    "active" => entry.active,
                    "inactive" => !entry.active,
                    "actinact" => self
                        .actinact
                        .as_ref()
                        .unwrap()
                        .contains(&entry.name.to_ascii_lowercase()),
                    "requested" => entry.requested,
                    "unrequested" => !entry.requested,
                    "leaves" | "rleaves" => self
                        .leaves
                        .as_ref()
                        .unwrap()
                        .matches(entry, spec.name == "rleaves"),
                    _ => matcher.matches(&entry.name),
                };
                name_matches
                    && (version.is_empty()
                        || version == entry.version
                        || version == entry.full_version())
            })
            .map(|(i, _)| i)
            .collect();
        ensure!(
            !result.is_empty() || pseudo || glob || matches!(self.missing, Missing::Empty),
            "port {}{} is not registered as installed",
            spec.name,
            if version.is_empty() {
                String::new()
            } else {
                format!(" @{version}")
            }
        );
        Ok(result)
    }
    fn dependency_atom(
        &mut self,
        name: &str,
        recursive: bool,
        modifiers: usize,
    ) -> Result<BTreeSet<usize>> {
        ensure!(!name.is_empty(), "dependent selector requires a port name");
        ensure!(
            !name.contains(['@', ':', '/', '*', '?', '[', ']']),
            "dependent selectors require an exact port name; put version filters after the selector"
        );
        let mut spec = PortSpec::named(name);
        for token in &self.tokens[modifiers..self.cursor] {
            spec.parse_modifier(token)?;
        }
        if !self
            .entries
            .iter()
            .any(|entry| entry.name.eq_ignore_ascii_case(name))
        {
            return Ok(BTreeSet::new());
        }
        if self.dependents.is_none() {
            let registry = self
                .registry
                .ok_or_else(|| anyhow::anyhow!("dependent selectors require an open registry"))?;
            self.dependents = Some(Graph::build(registry, self.entries)?);
        }
        let graph = self.dependents.as_ref().unwrap();
        let mut names = HashSet::new();
        if recursive {
            graph.walk(name, false, |entry, _, _| {
                names.insert(entry.name.to_ascii_lowercase());
                Ok(())
            })?;
        } else {
            names.extend(
                graph
                    .direct(name)
                    .map(|entry| entry.name.to_ascii_lowercase()),
            );
        }
        // Upstream dependency selectors yield names, so selecting one name includes
        // all its installed images, even if only one image records the dependency.
        let version = spec.composite_version();
        Ok(self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                names.contains(&entry.name.to_ascii_lowercase())
                    && (version.is_empty()
                        || version == entry.version
                        || version == entry.full_version())
            })
            .map(|(i, _)| i)
            .collect())
    }
}
