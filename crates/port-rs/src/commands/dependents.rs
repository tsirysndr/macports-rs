use crate::registry::{self, Registry, dependents::Graph};
use anyhow::{Result, ensure};
use clap::Args;
use std::{collections::HashSet, io::Write, path::Path};

#[derive(Args)]
pub struct Dependents {
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}
#[derive(Args)]
pub struct RecursiveDependents {
    #[command(flatten)]
    pub(super) selection: Dependents,
    /// Repeat shared subtrees and include versions; annotate cycles
    #[arg(long)]
    pub(super) full: bool,
}

pub(super) fn run(
    out: &mut impl Write,
    path: Option<&Path>,
    args: Dependents,
    recursive: Option<bool>,
    quiet: bool,
    verbose: bool,
) -> Result<()> {
    let specs = registry::specifications(&args.ports)?;
    for spec in &specs {
        ensure!(
            !spec.name.contains(['*', '?', '[', ']'])
                && !matches!(
                    spec.name.as_str(),
                    "all" | "installed" | "active" | "inactive" | "requested" | "unrequested"
                ),
            "dependent queries currently require explicit port names; registry selectors and globs are not implemented"
        );
    }
    let registry = Registry::open(path)?;
    let entries = registry
        .as_ref()
        .map(Registry::entries)
        .transpose()?
        .unwrap_or_default();
    let roots = specs
        .iter()
        .map(|spec| {
            let version = spec.composite_version();
            let matches: Vec<_> = entries
                .iter()
                .filter(|entry| {
                    entry.name.eq_ignore_ascii_case(&spec.name)
                        && (version.is_empty()
                            || version == entry.version
                            || version == entry.full_version())
                })
                .collect();
            matches
                .iter()
                .find(|entry| entry.active)
                .copied()
                .or_else(|| matches.first().copied())
                .ok_or_else(|| anyhow::anyhow!("port {} is not registered as installed", spec.name))
        })
        .collect::<Result<Vec<_>>>()?;
    let graph = Graph::build(registry.as_ref().unwrap(), &entries)?;
    let mut seen = HashSet::new();
    for root in roots {
        if graph.direct(&root.name).next().is_none() {
            if !quiet {
                writeln!(out, "{} has no dependents.", root.name)?;
            }
        } else if let Some(full) = recursive {
            if !quiet {
                writeln!(out, "The following ports are dependent on {}:", root.name)?;
            }
            graph.walk(&root.name, full, |entry, depth, cyclic| {
                let version = if full || verbose {
                    format!(" @{}", entry.full_version())
                } else {
                    String::new()
                };
                writeln!(
                    out,
                    "{:indent$}{}{version}{}",
                    "",
                    entry.name,
                    if cyclic { " (cyclic dependency)" } else { "" },
                    indent = depth * 2
                )?;
                Ok(())
            })?;
        } else {
            for entry in graph.direct(&root.name) {
                if !seen.insert(entry.name.clone()) && (quiet || !verbose) {
                    continue;
                }
                if quiet {
                    writeln!(out, "{}", entry.name)?;
                } else if verbose {
                    writeln!(
                        out,
                        "{} @{} depends on {} (by port:)",
                        entry.name,
                        entry.full_version(),
                        root.name
                    )?;
                } else {
                    writeln!(out, "{} depends on {}", entry.name, root.name)?;
                }
            }
        }
    }
    Ok(())
}
