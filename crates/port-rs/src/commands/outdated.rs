use crate::{
    index::{Index, Port},
    registry::{self, Entry},
    source,
};
use anyhow::{Context, Result};
use clap::Args;
use std::{cmp::Ordering, io::Write, path::Path};

#[derive(Args)]
pub struct Outdated {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}

pub(super) fn run(
    out: &mut impl Write,
    path: Option<&Path>,
    args: Outdated,
    quiet: bool,
    verbose: bool,
    load_index: impl FnOnce(&[&str]) -> Result<Index>,
) -> Result<()> {
    let registry = registry::Registry::open(path)?;
    let entries = registry
        .as_ref()
        .map(registry::Registry::entries)
        .transpose()?
        .unwrap_or_default();
    // Only installed ports are looked up, so only their records are parsed.
    let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    let mut load_index = Some(load_index);
    let mut cached_index = None;
    let selected: Vec<_> = registry::expression::select(
        &entries,
        &args.ports,
        registry::expression::Missing::Empty,
        registry.as_ref(),
        || {
            let index =
                cached_index.insert(load_index.take().expect("index loader used once")(&names)?);
            Ok(registry::expression::obsolete_names(&entries, index))
        },
    )?
    .into_iter()
    .filter(|entry| entry.active)
    .collect();
    if selected.is_empty() {
        if !quiet {
            writeln!(
                out,
                "{}",
                if args.ports.is_empty() {
                    "No ports are installed."
                } else {
                    "None of the specified ports are outdated."
                }
            )?;
        }
        return Ok(());
    }
    let index = match cached_index {
        Some(index) => index,
        None => load_index.take().expect("index loader used once")(&names)?,
    };
    let mut lines = Vec::new();
    let mut platform = None;
    for entry in selected {
        let Some(port) = index.lookup(&entry.name) else {
            continue;
        };
        if !port.fields.contains_key("version") {
            eprintln!("Warning: {} has no version field", port.name);
            continue;
        }
        let (mut order, mut reason) =
            compare(entry, port).with_context(|| format!("{}: outdated comparison", entry.name))?;
        if order == Ordering::Equal
            && !matches!(entry.platform.as_str(), "any" | "" | "0")
            && !matches!(entry.os_major.as_str(), "" | "0")
        {
            if platform.is_none() {
                platform = Some(source::detect_platform()?);
            }
            let host = platform.as_ref().unwrap();
            if entry.platform != "darwin"
                || (entry.os_major != "any"
                    && entry
                        .os_major
                        .parse::<u32>()
                        .context("invalid registry OS major")?
                        != host.darwin_major)
            {
                order = Ordering::Less;
                reason = format!(
                    " (platform {} {} != darwin {})",
                    entry.platform, entry.os_major, host.darwin_major
                );
            }
        }
        if order == Ordering::Less || (verbose && order == Ordering::Greater) {
            let relation = if order == Ordering::Less { '<' } else { '>' };
            let versions = format!(
                "{}_{} {relation} {} {reason}",
                entry.version,
                entry.revision,
                port.version()
            );
            lines.push(format!(
                "{:<30} {:<24} {:1}",
                port.name,
                versions,
                if order == Ordering::Greater { "!" } else { "" }
            ));
        }
    }
    if !quiet {
        writeln!(
            out,
            "{}",
            if lines.is_empty() {
                "No installed ports are outdated."
            } else {
                "The following installed ports are outdated:"
            }
        )?;
    }
    for line in lines {
        writeln!(out, "{line}")?;
    }
    Ok(())
}
fn number(raw: &str) -> Result<i64> {
    if raw.is_empty() {
        Ok(0)
    } else {
        raw.parse().context("invalid epoch or revision")
    }
}
fn compare(entry: &Entry, port: &Port) -> Result<(Ordering, String)> {
    let installed_epoch = number(&entry.epoch)?;
    let latest_epoch = number(port.get("epoch"))?;
    let latest_revision = number(port.get("revision"))?;
    let mut order = registry::compare_versions(&entry.version, port.get("version"))
        .then_with(|| entry.revision.cmp(&latest_revision));
    let epoch = installed_epoch.cmp(&latest_epoch);
    let mut reason = String::new();
    if epoch != Ordering::Equal && entry.version != port.get("version") {
        if (order != Ordering::Less && epoch == Ordering::Less)
            || (order != Ordering::Greater && epoch == Ordering::Greater)
        {
            reason = format!(
                " (epoch {installed_epoch} {} {latest_epoch})",
                if epoch == Ordering::Less { '<' } else { '>' }
            );
        }
        order = epoch;
    }
    Ok((order, reason))
}

/// Active registry entries older than the index (or built for another
/// platform), as `port outdated` reports them, for `port upgrade`.
pub(super) fn outdated_entries(
    path: Option<&Path>,
    ports: &[String],
    index: &Index,
) -> Result<Vec<Entry>> {
    let registry = registry::Registry::open(path)?;
    let entries = registry
        .as_ref()
        .map(registry::Registry::entries)
        .transpose()?
        .unwrap_or_default();
    let selected: Vec<_> = registry::expression::select(
        &entries,
        ports,
        registry::expression::Missing::Empty,
        registry.as_ref(),
        || Ok(registry::expression::obsolete_names(&entries, index)),
    )?
    .into_iter()
    .filter(|entry| entry.active)
    .collect();
    let mut result = Vec::new();
    let mut platform = None;
    for entry in selected {
        let Some(port) = index.lookup(&entry.name) else {
            continue;
        };
        let (mut order, _) = compare(entry, port)?;
        if order == Ordering::Equal
            && !matches!(entry.platform.as_str(), "any" | "" | "0")
            && !matches!(entry.os_major.as_str(), "" | "0" | "any")
        {
            if platform.is_none() {
                platform = Some(source::detect_platform()?);
            }
            let host = platform.as_ref().unwrap();
            if entry.platform != "darwin"
                || entry.os_major.parse::<u32>().ok() != Some(host.darwin_major)
            {
                order = Ordering::Less;
            }
        }
        if order == Ordering::Less {
            result.push(entry.clone());
        }
    }
    Ok(result)
}
