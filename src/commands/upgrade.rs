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
    let options = crate::options::get();
    let mut outdated = super::outdated::outdated_entries(Some(registry_path), &args.ports, index)?;
    if options.dependents {
        outdated.extend(outdated_dependents(registry_path, index, &outdated)?);
    }
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
    super::install_exec::run_specs(out, index, Some(registry_path), specs, quiet, root, true)?;
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
    if args.uninstall_inactive || options.uninstall_old {
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

/// `-R`: outdated ports that depend, directly or not, on the selected ones.
fn outdated_dependents(
    registry_path: &Path,
    index: &Index,
    selected: &[registry::Entry],
) -> Result<Vec<registry::Entry>> {
    let Some(reader) = registry::Registry::open(Some(registry_path))? else {
        return Ok(Vec::new());
    };
    let entries = reader.entries()?;
    let edges = reader.dependent_edges()?;
    let mut names: std::collections::HashSet<String> = selected
        .iter()
        .map(|entry| entry.name.to_ascii_lowercase())
        .collect();
    loop {
        let before = names.len();
        for (dependency, id) in &edges {
            if names.contains(&dependency.to_ascii_lowercase())
                && let Some(entry) = entries.iter().find(|entry| entry.id == *id)
            {
                names.insert(entry.name.to_ascii_lowercase());
            }
        }
        if names.len() == before {
            break;
        }
    }
    let chosen: std::collections::HashSet<i64> = selected.iter().map(|entry| entry.id).collect();
    Ok(
        super::outdated::outdated_entries(Some(registry_path), &[], index)?
            .into_iter()
            .filter(|entry| {
                !chosen.contains(&entry.id) && names.contains(&entry.name.to_ascii_lowercase())
            })
            .collect(),
    )
}
