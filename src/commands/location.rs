use crate::registry::{self, Registry};
use anyhow::{Result, ensure};
use clap::Args;
use std::{io::Write, path::Path};

#[derive(Args)]
pub struct Location {
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}

pub(super) fn run(
    out: &mut impl Write,
    path: Option<&Path>,
    args: Location,
    quiet: bool,
) -> Result<()> {
    let specs = registry::specifications(&args.ports)?;
    for spec in &specs {
        ensure!(
            !spec.name.contains(['*', '?', '[', ']'])
                && !matches!(
                    spec.name.as_str(),
                    "all" | "installed" | "active" | "inactive" | "requested" | "unrequested"
                ),
            "location currently requires explicit port names; registry selectors and globs are not implemented"
        );
    }
    let registry = Registry::open(path)?;
    let entries = registry
        .as_ref()
        .map(Registry::entries)
        .transpose()?
        .unwrap_or_default();
    let mut results = Vec::new();
    for spec in specs {
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
        ensure!(
            !matches.is_empty(),
            "port {}{} is not registered as installed",
            spec.name,
            if version.is_empty() {
                String::new()
            } else {
                format!(" @{version}")
            }
        );
        ensure!(
            matches.len() == 1,
            "multiple versions of {} are installed; specify the full version as recorded in the port registry: {}",
            spec.name,
            matches
                .iter()
                .map(|entry| format!(
                    "@{}{}",
                    entry.full_version(),
                    if entry.active { " (active)" } else { "" }
                ))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let entry = matches[0];
        // Related entry and location reads use the same registry snapshot.
        results.push((entry, registry.as_ref().unwrap().location(entry)?));
    }
    for (entry, location) in results {
        if !quiet {
            writeln!(
                out,
                "Port {} {} is installed as an image in:",
                entry.name,
                entry.full_version()
            )?;
        }
        writeln!(out, "{location}")?;
    }
    Ok(())
}
