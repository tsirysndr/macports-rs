use super::size::bytesize;
use crate::registry::{self, Entry, Registry};
use anyhow::{Result, ensure};
use clap::Args;
use std::{io::Write, path::Path};

#[derive(Args)]
pub struct Contents {
    /// Display the current filesystem size alongside each registered path
    #[arg(long)]
    size: bool,
    /// Size unit (B, KiB, kB, MiB, MB, GiB, GB; upstream abbreviations accepted)
    #[arg(long)]
    units: Option<String>,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}

pub(super) fn run(
    out: &mut impl Write,
    path: Option<&Path>,
    args: Contents,
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
            "contents currently requires explicit port names, optionally qualified by version and variants; registry selectors and globs are not implemented"
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
        let matches: Vec<&Entry> = entries
            .iter()
            .filter(|entry| {
                entry.name.eq_ignore_ascii_case(&spec.name)
                    && (version.is_empty()
                        || version == entry.version
                        || version == entry.full_version())
            })
            .collect();
        let chosen = if let Some(active) = matches.iter().find(|entry| entry.active) {
            *active
        } else {
            ensure!(
                !matches.is_empty(),
                "port {} is not registered as installed",
                spec.name
            );
            ensure!(
                matches.len() == 1,
                "multiple inactive versions of {} are installed; specify the full version as recorded in the port registry: {}",
                spec.name,
                matches
                    .iter()
                    .map(|entry| format!("@{}", entry.full_version()))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            matches[0]
        };
        // A matching entry can only exist when the registry was successfully opened.
        let files = registry.as_ref().unwrap().files(chosen)?;
        results.push((chosen, files));
    }
    for (entry, files) in results {
        if !quiet {
            writeln!(
                out,
                "Port {} @{} {}",
                entry.name,
                entry.full_version(),
                if files.is_empty() {
                    "does not have any files registered."
                } else {
                    "contains:"
                }
            )?;
        }
        for file in files {
            if args.size {
                let size = std::fs::metadata(&file)
                    .map(|metadata| bytesize(metadata.len(), args.units.as_deref()))
                    .unwrap_or_else(|_| "-1".into());
                writeln!(out, "{size:>12} {file}")?;
            } else {
                writeln!(out, "  {file}")?;
            }
        }
    }
    Ok(())
}
