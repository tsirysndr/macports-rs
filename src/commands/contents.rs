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

fn bytesize(bytes: u64, unit: Option<&str>) -> String {
    let unit = match unit.unwrap_or("") {
        "" if bytes > 1_073_741_824 => "GiB",
        "" if bytes > 1_048_576 => "MiB",
        "" if bytes > 1024 => "KiB",
        "" => "B",
        "K" | "Ki" => "KiB",
        "k" => "kB",
        "Mi" => "MiB",
        "M" => "MB",
        "Gi" => "GiB",
        "G" => "GB",
        unit => unit,
    };
    let divisor = match unit {
        "KiB" => 1024.0,
        "kB" => 1000.0,
        "MiB" => 1_048_576.0,
        "MB" => 1_000_000.0,
        "GiB" => 1_073_741_824.0,
        "GB" => 1_000_000_000.0,
        "B" => return format!("{bytes} B"),
        _ => {
            eprintln!("Warning: Unknown file size unit '{unit}' specified");
            return format!("{bytes} B");
        }
    };
    let value = bytes as f64 / divisor;
    if value.fract() == 0.0 {
        format!("{value:.1} {unit}")
    } else {
        format!("{value:.2} {unit}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn size_units_preserve_upstream_thresholds_and_abbreviations() {
        assert_eq!(bytesize(1024, None), "1024 B");
        assert_eq!(bytesize(1025, None), "1.00 KiB");
        assert_eq!(bytesize(1_048_576, None), "1024.0 KiB");
        assert_eq!(bytesize(1_073_741_824, None), "1024.0 MiB");
        for (unit, expected) in [
            ("K", "1.0 KiB"),
            ("Ki", "1.0 KiB"),
            ("k", "1.02 kB"),
            ("B", "1024 B"),
        ] {
            assert_eq!(bytesize(1024, Some(unit)), expected);
        }
        assert_eq!(bytesize(1_000_000, Some("M")), "1.0 MB");
        assert_eq!(bytesize(0, Some("Gi")), "0.0 GiB");
    }
}
