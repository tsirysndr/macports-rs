//! Global variants.conf choices; these are defaults beneath explicit requests.
use crate::spec::PortSpec;
use anyhow::{Context, Result};
use std::{collections::BTreeMap, path::Path};

#[derive(Default, Debug)]
pub struct Variants {
    pub choices: BTreeMap<String, bool>,
    pub warnings: Vec<String>,
}
impl Variants {
    pub fn read(path: Option<&Path>) -> Result<Self> {
        let Some(path) = path else {
            return Ok(Self::default());
        };
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("cannot read variants configuration {}", path.display())
                });
            }
        };
        Ok(Self::parse(&source, &path.display().to_string()))
    }
    pub fn parse(source: &str, label: &str) -> Self {
        let mut result = Self::default();
        for line in source.lines() {
            let line = line.trim_end();
            if line.is_empty() || line.trim_start_matches([' ', '\t']).starts_with('#') {
                continue;
            }
            // Tcl split retains empty elements from repeated/leading separators.
            for value in line.split([' ', '\t']) {
                let valid = value.len() > 1
                    && matches!(value.as_bytes()[0], b'+' | b'-')
                    && value.as_bytes()[1..]
                        .iter()
                        .all(|c| c.is_ascii_alphanumeric() || b"_-+.".contains(c));
                if valid {
                    result
                        .choices
                        .entry(value[1..].into())
                        .or_insert(value.starts_with('+'));
                } else {
                    result.warnings.push(format!(
                        "{label} specifies invalid variant syntax '{value}', ignored."
                    ));
                }
            }
        }
        result
    }
    /// Explicit per-port selections win; the resulting spec also carries global
    /// choices through dependency inheritance in the installation planner.
    pub fn apply(&self, spec: &mut PortSpec) {
        for (name, enabled) in &self.choices {
            spec.variants.entry(name.clone()).or_insert(*enabled);
        }
    }
}
