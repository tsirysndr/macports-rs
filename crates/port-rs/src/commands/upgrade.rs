//! `port upgrade`: rebuild outdated active ports with their installed variants,
//! keeping requested flags; `-u` uninstalls the replaced inactive images.
use crate::{index::Index, registry, registry::write::Writer, spec::PortSpec};
use anyhow::{Context, Result};
use clap::Args;
use std::{collections::HashMap, io::Write, path::Path};

#[derive(Args)]
pub struct Upgrade {
    /// Uninstall the replaced, now inactive images
    #[arg(short = 'u')]
    uninstall_inactive: bool,
    /// Rebuild and reinstall the ports even if they are up to date
    #[arg(long)]
    force: bool,
    /// Upgrade ports whose installed variants differ from the requested ones
    #[arg(long = "enforce-variants")]
    enforce_variants: bool,
    /// Upgrade replaced ports instead of installing their replacements
    #[arg(long = "no-replace")]
    no_replace: bool,
    /// Skip checking for broken binaries after upgrading
    #[arg(long = "no-rev-upgrade")]
    no_rev_upgrade: bool,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}

/// Port selectors, and the variants requested after each port name.
fn split_variations(tokens: &[String]) -> Result<(Vec<String>, HashMap<String, PortSpec>)> {
    let mut selectors = Vec::new();
    let mut called: HashMap<String, PortSpec> = HashMap::new();
    let mut current: Option<String> = None;
    for token in tokens {
        if let Some(name) = current
            .as_ref()
            .filter(|_| token.starts_with(['+', '-']) && token.len() > 1)
        {
            called
                .entry(name.to_ascii_lowercase())
                .or_insert_with(|| PortSpec::named(name))
                .parse_modifier(token)?;
        } else {
            current = Some(token.clone());
            selectors.push(token.clone());
        }
    }
    Ok((selectors, called))
}

/// Installed variants, then the request's negative variants, then the
/// variants named on the command line.
fn upgrade_spec(
    entry: &registry::Entry,
    name: &str,
    called: Option<&PortSpec>,
) -> Result<PortSpec> {
    let mut spec = PortSpec::named(name);
    if !entry.variants.is_empty() {
        spec.parse_modifier(&entry.variants)?;
    }
    if !entry.requested_variants.is_empty() {
        let mut requested = PortSpec::named(name);
        requested.parse_modifier(&entry.requested_variants)?;
        for (variant, enabled) in requested.variants {
            if !enabled {
                spec.variants.insert(variant, false);
            }
        }
    }
    if let Some(called) = called {
        spec.variants.extend(called.variants.clone());
    }
    Ok(spec)
}

fn canonical(spec: &PortSpec) -> String {
    spec.variants
        .iter()
        .filter(|(_, enabled)| **enabled)
        .map(|(name, _)| format!("+{name}"))
        .collect()
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
    let progress = crate::progress::Progress::new(quiet);
    let (selectors, called) = split_variations(&args.ports)?;
    // `outdated` selects every outdated port; the registry alone cannot.
    let names: Vec<String> = selectors
        .iter()
        .filter(|token| token.as_str() != "outdated")
        .cloned()
        .collect();
    let mut outdated = super::outdated::outdated_entries(Some(registry_path), &names, index)?;
    if options.dependents {
        outdated.extend(outdated_dependents(registry_path, index, &outdated)?);
    }
    // Ports that are current but forced, replaced, or need other variants.
    if !names.is_empty()
        && let Some(registry) = registry::Registry::open(Some(registry_path))?
    {
        let entries = registry.entries()?;
        let selected = registry::expression::select(
            &entries,
            &names,
            registry::expression::Missing::Empty,
            Some(&registry),
            || Ok(registry::expression::obsolete_names(&entries, index)),
        )?;
        for entry in selected.into_iter().filter(|entry| entry.active) {
            if outdated.iter().any(|known| known.id == entry.id) {
                continue;
            }
            let replaced = index
                .lookup(&entry.name)
                .is_some_and(|port| !port.get("replaced_by").is_empty());
            let spec = upgrade_spec(
                entry,
                &entry.name,
                called.get(&entry.name.to_ascii_lowercase()),
            )?;
            let different = canonical(&spec) != entry.variants;
            if args.force || (replaced && !args.no_replace) || (different && args.enforce_variants)
            {
                outdated.push(entry.clone());
            } else if different {
                progress.warning(&format!(
                    "Skipping upgrade since {} {} is current, even though installed variants \"{}\" do not match \"{}\". Use 'upgrade --enforce-variants' to switch to the requested variants.",
                    entry.name,
                    entry.full_version(),
                    entry.variants,
                    canonical(&spec)
                ));
            }
        }
    }
    if outdated.is_empty() {
        if !quiet {
            writeln!(out, "Nothing to upgrade.")?;
        }
        return Ok(());
    }
    let mut specs = Vec::new();
    // (old entry, the port that replaces it)
    let mut targets = Vec::new();
    for entry in &outdated {
        let mut name = entry.name.clone();
        if let Some(port) = index.lookup(&entry.name) {
            let replacement = port.get("replaced_by");
            if !replacement.is_empty() && !args.no_replace {
                progress.phase(&format!("{} is replaced by {replacement}", entry.name));
                name = crate::query::lookup(index, replacement)
                    .with_context(|| format!("No port {replacement} found."))?
                    .name
                    .clone();
            }
        }
        specs.push(upgrade_spec(
            entry,
            &name,
            called.get(&entry.name.to_ascii_lowercase()),
        )?);
        targets.push((entry.clone(), name));
    }
    let mode = super::install_exec::Mode {
        rebuild: args.force,
        ..super::install_exec::Mode::INSTALL
    };
    super::install_exec::run_specs(out, index, Some(registry_path), specs, quiet, root, mode)?;
    // Keep requested flags, retire replaced ports, and optionally remove the
    // replaced images.
    let entries = registry::Registry::open(Some(registry_path))?
        .map(|registry| registry.entries())
        .transpose()?
        .unwrap_or_default();
    let mut writer = Writer::open(registry_path)?;
    let mut transaction = writer.transaction()?;
    for (old, name) in &targets {
        if let Some(new) = entries
            .iter()
            .find(|entry| entry.active && entry.name.eq_ignore_ascii_case(name))
        {
            transaction.set_requested(new.id, old.requested)?;
        }
    }
    transaction.commit()?;
    drop(writer);
    for (old, name) in &targets {
        if !old.name.eq_ignore_ascii_case(name)
            && let Some(current) = entries
                .iter()
                .find(|entry| entry.id == old.id && entry.active)
        {
            super::lifecycle::deactivate_entry(registry_path, current, &progress)?;
        }
    }
    if args.uninstall_inactive || options.uninstall_old {
        let entries = registry::read(Some(registry_path))?;
        for (old, _) in &targets {
            if let Some(current) = entries
                .iter()
                .find(|entry| entry.id == old.id && !entry.active)
            {
                super::lifecycle::uninstall_entry(registry_path, current, &progress)?;
            }
        }
    }
    if !args.no_rev_upgrade && !options.nodeps {
        super::rev_upgrade::autorun(out, index, registry_path, quiet, root)?;
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
