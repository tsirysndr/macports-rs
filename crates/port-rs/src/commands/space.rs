use super::size::bytesize;
use crate::registry::{self, Registry};
use anyhow::{Result, ensure};
use clap::Args;
use std::{collections::HashSet, io::Write, os::unix::fs::MetadataExt, path::Path};

#[derive(Args)]
pub struct Space {
    /// Display only the combined total
    #[arg(long)]
    total: bool,
    /// Size unit (B, KiB, kB, MiB, MB, GiB, GB; upstream abbreviations accepted)
    #[arg(long)]
    units: Option<String>,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}

pub(super) fn run(out: &mut impl Write, path: Option<&Path>, args: Space) -> Result<()> {
    let specs = registry::specifications(&args.ports)?;
    for spec in &specs {
        ensure!(
            !spec.name.contains(['*', '?', '[', ']'])
                && !matches!(
                    spec.name.as_str(),
                    "all" | "installed" | "active" | "inactive" | "requested" | "unrequested"
                ),
            "space currently requires explicit port names; registry selectors and globs are not implemented"
        );
    }
    let registry = Registry::open(path)?;
    let entries = registry
        .as_ref()
        .map(Registry::entries)
        .transpose()?
        .unwrap_or_default();
    let mut lines = Vec::new();
    let mut messages = Vec::new();
    let mut total = 0u64;
    for spec in &specs {
        let Some(entry) = entries
            .iter()
            .find(|entry| entry.active && entry.name.eq_ignore_ascii_case(&spec.name))
        else {
            messages.push(format!("Port {} is not active.", spec.name));
            continue;
        };
        let active_version = format!("{}_{}", entry.version, entry.revision);
        if let Some(version) = &spec.version
            && version != &active_version
        {
            messages.push(format!(
                "Warning: Active version of {} is not {} but {active_version}",
                entry.name, version
            ));
        }
        let files = registry.as_ref().unwrap().files(entry)?;
        if files.is_empty() {
            messages.push(format!(
                "Port {} does not have any files registered.",
                spec.name
            ));
            continue;
        }
        let size = file_sizes(&files)?;
        total = total
            .checked_add(size)
            .ok_or_else(|| anyhow::anyhow!("combined file sizes exceed u64"))?;
        if !args.total {
            let version = if spec.version.is_none() {
                String::new()
            } else {
                format!(" @{}", entry.full_version())
            };
            lines.push(format!(
                "{} {}{version}",
                bytesize(size, args.units.as_deref()),
                spec.name
            ));
        }
    }
    if specs.len() > 1 || args.total {
        lines.push(format!("{} total", bytesize(total, args.units.as_deref())));
    }
    for message in messages {
        eprintln!("{message}");
    }
    for line in lines {
        writeln!(out, "{line}")?;
    }
    Ok(())
}

fn file_sizes(files: &[String]) -> Result<u64> {
    let mut seen = HashSet::new();
    let mut size = 0u64;
    for file in files {
        // lstat measures the link itself, including dangling links. Unreadable or
        // removed files are skipped, as in the native action. Deduplicate per port.
        let Ok(metadata) = std::fs::symlink_metadata(file) else {
            continue;
        };
        if metadata.nlink() == 1 || seen.insert((metadata.dev(), metadata.ino())) {
            size = size
                .checked_add(metadata.len())
                .ok_or_else(|| anyhow::anyhow!("file sizes exceed u64"))?;
        }
    }
    Ok(size)
}
