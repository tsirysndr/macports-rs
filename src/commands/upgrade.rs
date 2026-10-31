//! `port upgrade`: rebuild outdated active ports with their installed variants,
//! keeping requested flags; `-u` uninstalls the replaced inactive images.
use crate::{index::Index, registry, registry::write::Writer, spec::PortSpec};
use anyhow::Result;
use clap::Args;
use std::{io::Write, path::Path};

#[derive(Args)]
pub struct Upgrade {
    /// Uninstall the replaced, now inactive images
    #[arg(short = 'u')]
    uninstall_inactive: bool,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}

pub(super) fn run(
    out: &mut impl Write,
    index: &Index,
    registry_path: &Path,
    args: Upgrade,
    quiet: bool,
    root: &Path,
) -> Result<()> {
    let outdated = super::outdated::outdated_entries(Some(registry_path), &args.ports, index)?;
    if outdated.is_empty() {
        if !quiet {
            writeln!(out, "Nothing to upgrade.")?;
        }
        return Ok(());
    }
    let mut specs = Vec::new();
    for entry in &outdated {
        let mut spec = PortSpec::named(&entry.name);
        for variant in entry.variants.split('+').filter(|v| !v.is_empty()) {
            spec.parse_modifier(&format!("+{variant}"))?;
        }
        specs.push(spec);
    }
    super::install_exec::run_specs(out, index, Some(registry_path), specs, quiet, root)?;
    // Restore requested flags and optionally remove the replaced images.
    let entries = registry::Registry::open(Some(registry_path))?
        .map(|registry| registry.entries())
        .transpose()?
        .unwrap_or_default();
    let mut writer = Writer::open(registry_path)?;
    let mut transaction = writer.transaction()?;
    for old in &outdated {
        if let Some(new) = entries
            .iter()
            .find(|entry| entry.active && entry.name.eq_ignore_ascii_case(&old.name))
        {
            transaction.set_requested(new.id, old.requested)?;
        }
    }
    transaction.commit()?;
    drop(writer);
    if args.uninstall_inactive {
        let progress = crate::progress::Progress::new(quiet);
        for old in &outdated {
            let current = entries
                .iter()
                .find(|entry| entry.id == old.id && !entry.active);
            if let Some(current) = current {
                super::lifecycle::uninstall_entry(registry_path, current, &progress)?;
            }
        }
    }
    Ok(())
}
