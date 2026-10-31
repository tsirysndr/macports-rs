//! `port restore` and `port migrate` (macports::restore, macports::migrate):
//! reinstall the ports recorded in a registry snapshot.
use super::snapshot::{self, Diff};
use crate::{
    index::Index,
    progress::Progress,
    registry::{Entry, Registry, snapshot::SnapshotPort},
    spec::PortSpec,
};
use anyhow::{Context, Result, bail};
use clap::Args;
use std::{collections::BTreeMap, io::Write, path::Path};

#[derive(Args)]
pub struct Restore {
    /// Restore this snapshot
    #[arg(long = "snapshot-id", value_name = "ID")]
    snapshot_id: Option<i64>,
    /// Restore the most recent snapshot
    #[arg(long)]
    last: bool,
    /// Also restore unrequested ports
    #[arg(long)]
    all: bool,
}

#[derive(Args)]
pub struct Migrate {
    /// Continue a migration after MacPorts base was updated
    #[arg(long = "continue")]
    continuation: bool,
    /// Also restore unrequested ports
    #[arg(long)]
    all: bool,
}

/// Registry entries with their (dependency name, dependent id) edges.
type Installed = (Vec<Entry>, Vec<(String, i64)>);

fn entries(path: &Path) -> Result<Installed> {
    match Registry::open(Some(path))? {
        Some(registry) => Ok((registry.entries()?, registry.dependent_edges()?)),
        None => Ok((Vec::new(), Vec::new())),
    }
}

/// Dependents before their dependencies.
fn deactivation_order(mut remaining: Vec<Entry>, edges: &[(String, i64)]) -> Vec<Entry> {
    let mut ordered = Vec::new();
    while !remaining.is_empty() {
        let position = remaining
            .iter()
            .position(|entry| {
                !edges.iter().any(|(name, id)| {
                    name.eq_ignore_ascii_case(&entry.name)
                        && remaining
                            .iter()
                            .any(|other| other.id == *id && other.id != entry.id)
                })
            })
            .unwrap_or(0);
        ordered.push(remaining.remove(position));
    }
    ordered
}

fn deactivate_all(path: &Path, progress: &Progress) -> Result<()> {
    let (entries, edges) = entries(path)?;
    let active: Vec<Entry> = entries.into_iter().filter(|entry| entry.active).collect();
    for entry in deactivation_order(active, &edges) {
        if let Err(err) = super::lifecycle::deactivate_entry(path, &entry, progress) {
            progress.warning(&format!("Failed to deactivate {}: {err:#}", entry.name));
        }
    }
    Ok(())
}

fn spec(port: &SnapshotPort) -> Result<PortSpec> {
    let mut spec = PortSpec::named(&port.port_name);
    let text = &port.requested_variants;
    let mut rest = text.as_str();
    while let Some(start) = rest.find(['+', '-']) {
        let tail = &rest[start + 1..];
        let end = tail.find(['+', '-']).unwrap_or(tail.len());
        spec.parse_modifier(&rest[start..start + 1 + end])?;
        rest = &tail[end..];
    }
    Ok(spec)
}

fn line(port: &SnapshotPort) -> String {
    if port.requested_variants.is_empty() {
        format!(" - {} \n", port.port_name)
    } else {
        format!(" - {}\n", port.port_name)
    }
}

fn reason(failure: &(bool, String)) -> String {
    match failure {
        (true, reason) => format!("   Skipped because its {reason}\n"),
        (false, reason) => format!("   Failed: {reason}\n"),
    }
}

/// Restore a snapshot by id (or the newest); returns whether any port failed.
#[allow(clippy::too_many_arguments)]
fn restore(
    out: &mut impl Write,
    index: &Index,
    path: &Path,
    id: Option<i64>,
    all: bool,
    quiet: bool,
    root: &Path,
    progress: &Progress,
) -> Result<()> {
    let registry = Registry::open(Some(path))?.context("registry is missing")?;
    let snapshots = registry.snapshots()?;
    let snapshot = match id {
        Some(id) => snapshots
            .iter()
            .find(|snapshot| snapshot.id == id)
            .with_context(|| format!("no snapshot with ID {id}"))?
            .clone(),
        None => snapshots
            .first()
            .context("There are no snapshots to restore. You must run 'sudo port snapshot' first.")?
            .clone(),
    };
    let ports = registry.snapshot_ports(snapshot.id, false)?;
    drop(registry);

    progress.phase_with_prefix("===>", "Deactivating all installed ports");
    deactivate_all(path, progress)?;
    progress.phase_with_prefix(
        "===>",
        &format!(
            "Restoring snapshot '{}' created at {}",
            snapshot.note, snapshot.created_at
        ),
    );
    let selected: Vec<&SnapshotPort> = ports
        .iter()
        .filter(|port| all || port.requested == 1)
        .collect();
    let mut failed: BTreeMap<String, (bool, String)> = BTreeMap::new();
    for (index_number, port) in selected.iter().enumerate() {
        let variants = if port.requested_variants.is_empty() {
            String::new()
        } else {
            format!(" {}", port.requested_variants)
        };
        progress.phase_with_prefix(
            "===>",
            &format!(
                "Restoring port {} of {}: {}{variants}",
                index_number + 1,
                selected.len(),
                port.port_name
            ),
        );
        let result = spec(port).and_then(|spec| {
            super::install_exec::run_specs(
                out,
                index,
                Some(path),
                vec![spec],
                quiet,
                root,
                port.state == "installed",
            )
        });
        if let Err(err) = result {
            failed.insert(
                port.port_name.clone(),
                (
                    false,
                    format!(
                        "Unable to execute target 'install' for port {}: {err:#}",
                        port.port_name
                    ),
                ),
            );
            continue;
        }
        let (entries, _) = entries(path)?;
        if let Some(entry) = entries
            .iter()
            .filter(|entry| entry.name.eq_ignore_ascii_case(&port.port_name))
            .max_by_key(|entry| entry.active)
            && entry.requested != (port.requested == 1)
        {
            let mut writer = crate::registry::write::Writer::open(path)?;
            let mut transaction = writer.transaction()?;
            transaction.set_requested(entry.id, port.requested == 1)?;
            transaction.commit()?;
        }
    }

    let (entries, _) = entries(path)?;
    let mut diff: Diff = snapshot::diff(&ports, &entries);
    if !all {
        diff.added.retain(|port| port.requested == 1);
        diff.removed.retain(|port| port.requested == 1);
        diff.changed.retain(|(port, _)| port.requested == 1);
    }
    let sort = |ports: &mut Vec<SnapshotPort>| ports.sort_by(|a, b| a.port_name.cmp(&b.port_name));
    sort(&mut diff.added);
    sort(&mut diff.removed);
    diff.changed
        .sort_by(|a, b| a.0.port_name.cmp(&b.0.port_name));
    let mut note = if failed.is_empty() {
        "Migration finished.\n".to_string()
    } else {
        "Migration finished with errors.\n".to_string()
    };
    if !diff.added.is_empty() {
        note.push_str("The following requested ports were additionally installed:\n");
        diff.added
            .iter()
            .for_each(|port| note.push_str(&line(port)));
    }
    if !diff.removed.is_empty() {
        note.push_str("The following ports could not be restored:\n");
        for port in &diff.removed {
            note.push_str(&line(port));
            if let Some(failure) = failed.get(&port.port_name) {
                note.push_str(&reason(failure));
            }
        }
    }
    let (partial, changed): (Vec<_>, Vec<_>) = diff
        .changed
        .iter()
        .partition(|(port, _)| failed.contains_key(&port.port_name));
    if !partial.is_empty() {
        note.push_str("The following ports could not be fully restored:\n");
        for (port, changes) in partial {
            note.push_str(&line(port));
            note.push_str(&reason(&failed[&port.port_name]));
            for (field, old, new) in changes {
                note.push_str(&format!("   {field} changed from '{old}' to '{new}'\n"));
            }
        }
    }
    if !changed.is_empty() {
        note.push_str("The following ports were restored with changes:\n");
        for (port, changes) in changed {
            note.push_str(&line(port));
            for (field, old, new) in changes {
                note.push_str(&format!("   {field} changed from '{old}' to '{new}'\n"));
            }
        }
    }
    progress.message(note.trim_end_matches('\n'));
    if !failed.is_empty() {
        bail!("restoring {} port(s) failed", failed.len());
    }
    Ok(())
}

pub(super) fn run(
    out: &mut impl Write,
    index: &Index,
    path: &Path,
    args: Restore,
    quiet: bool,
    root: &Path,
) -> Result<()> {
    let progress = Progress::new(quiet);
    let id = match (args.snapshot_id, args.last) {
        (Some(id), _) => Some(id),
        (None, true) => None,
        (None, false) => {
            let snapshots = Registry::open(Some(path))?
                .map(|registry| registry.snapshots())
                .transpose()?
                .unwrap_or_default();
            match snapshots.as_slice() {
                [] => bail!(
                    "There are no snapshots to restore. You must run 'sudo port snapshot' first."
                ),
                [only] => Some(only.id),
                many => {
                    let listing: Vec<String> = many
                        .iter()
                        .map(|s| format!("{}, created at {} (ID: {})", s.note, s.created_at, s.id))
                        .collect();
                    bail!(
                        "Multiple snapshots exist, specify which one to restore using --snapshot-id or use --last to restore the most recent one.\n{}",
                        listing.join("\n")
                    );
                }
            }
        }
    };
    restore(out, index, path, id, args.all, quiet, root, &progress)
}

pub(super) fn migrate(
    out: &mut impl Write,
    index: &Index,
    path: &Path,
    args: Migrate,
    quiet: bool,
    root: &Path,
) -> Result<()> {
    let progress = Progress::new(quiet);
    let _ = args.continuation;
    super::sync::run(quiet, false).context("Couldn't sync the ports tree")?;
    progress.message("Taking a snapshot of the current state...");
    let id = snapshot::create(out, path, "snapshot created for migration")
        .context("Failed to create a snapshot and migration cannot proceed.")?;
    let created = Registry::open(Some(path))?
        .context("registry is missing")?
        .snapshot(id)?
        .context("snapshot vanished")?;
    progress.message(&format!(
        "Done: Snapshot '{id}' : '{}' created at {}",
        created.note, created.created_at
    ));
    progress.message("Deactivating all ports...");
    deactivate_all(path, &progress)?;
    progress.message("Uninstalling ports that need to be reinstalled...");
    let (entries, edges) = entries(path)?;
    let platform = crate::source::detect_platform()?;
    let host = super::local::Host::new(root, Some(path), quiet)?;
    for entry in deactivation_order(entries, &edges) {
        let current = entry.platform == "any"
            || entry.platform == "darwin"
                && (entry.os_major == "any" || entry.os_major == platform.darwin_major.to_string());
        let archs_match = current
            && crate::query::lookup(index, &entry.name)
                .ok()
                .and_then(|port| {
                    let spec = spec(&SnapshotPort {
                        port_name: entry.name.clone(),
                        requested: 0,
                        state: String::new(),
                        variants: String::new(),
                        requested_variants: entry.requested_variants.clone(),
                        port_files: Vec::new(),
                    })
                    .ok()?;
                    let mut session = host.open(root, port, &spec).ok()?;
                    session.invoke("get_canonical_archs", &[]).ok()
                })
                .is_some_and(|archs| archs == entry.archs);
        if archs_match {
            continue;
        }
        progress.message(&format!("Uninstalling: {}", entry.name));
        if let Err(err) = super::lifecycle::uninstall_entry(path, &entry, &progress) {
            progress.error(&format!("Error uninstalling {}: {err:#}", entry.name));
        }
    }
    progress.message("Restoring ports...");
    restore(out, index, path, None, args.all, quiet, root, &progress)
}
