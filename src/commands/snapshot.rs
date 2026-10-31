//! `port snapshot` (macports::snapshot): record, list, compare, export and
//! import the set of active ports.
use crate::registry::{
    Entry, Registry,
    snapshot::{Snapshot, SnapshotPort},
    write::Writer,
};
use anyhow::{Context, Result, bail};
use clap::Args;
use std::{io::Write, path::Path};

#[derive(Args)]
pub struct SnapshotArgs {
    /// Record the active ports (the default)
    #[arg(long)]
    create: bool,
    /// Note stored with a new snapshot
    #[arg(long, num_args = 1..)]
    note: Option<Vec<String>>,
    /// List snapshots
    #[arg(long)]
    list: bool,
    /// Compare a snapshot with the installed ports
    #[arg(long, value_name = "ID")]
    diff: Option<i64>,
    /// With --diff, include unrequested ports
    #[arg(long)]
    all: bool,
    /// Delete a snapshot
    #[arg(long, value_name = "ID")]
    delete: Option<i64>,
    /// Write a snapshot as JSON to standard output
    #[arg(long, value_name = "ID")]
    export: Option<i64>,
    /// Import a snapshot from a JSON file
    #[arg(long, value_name = "FILE")]
    import: Option<String>,
}

/// Seconds since the epoch for a registry `YYYY-MM-DD HH:MM:SS` UTC time.
pub(super) fn utc_seconds(utc: &str) -> Option<i64> {
    let (date, time) = utc.split_once(' ')?;
    let mut date = date.split('-').map(|part| part.parse::<i32>());
    let mut time = time.split(':').map(|part| part.parse::<i32>());
    // SAFETY: an all-zero tm is a valid value to fill in.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = date.next()?.ok()? - 1900;
    tm.tm_mon = date.next()?.ok()? - 1;
    tm.tm_mday = date.next()?.ok()?;
    tm.tm_hour = time.next()?.ok()?;
    tm.tm_min = time.next()?.ok()?;
    tm.tm_sec = time.next()?.ok()?;
    // SAFETY: timegm only reads the initialized tm.
    Some(unsafe { libc::timegm(&mut tm) } as i64)
}

/// A registry UTC time shown in local time with its offset.
pub(super) fn local_time(utc: &str) -> String {
    let Some(seconds) = utc_seconds(utc) else {
        return utc.to_string();
    };
    let seconds = seconds as libc::time_t;
    // SAFETY: localtime_r and strftime write only into the provided buffers.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&seconds, &mut tm);
        let mut buffer = [0u8; 64];
        let length = libc::strftime(
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            c"%Y-%m-%d %T%z".as_ptr(),
            &tm,
        );
        String::from_utf8_lossy(&buffer[..length]).into_owned()
    }
}

fn current_os() -> (String, String) {
    let release = std::process::Command::new("/usr/bin/uname")
        .arg("-r")
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_default();
    (
        "darwin".into(),
        release.split('.').next().unwrap_or_default().to_string(),
    )
}

fn os_mismatch(entry: &Entry, os: &(String, String)) -> bool {
    entry.platform != "any"
        && (entry.platform != os.0 || entry.os_major != "any" && entry.os_major != os.1)
}

/// A changed field: name, snapshot value, current value.
pub(super) type Change = (String, String, String);

pub(super) struct Diff {
    pub added: Vec<SnapshotPort>,
    pub removed: Vec<SnapshotPort>,
    pub changed: Vec<(SnapshotPort, Vec<Change>)>,
}

pub(super) fn diff(snapshot: &[SnapshotPort], entries: &[Entry]) -> Diff {
    let os = current_os();
    let mut result = Diff {
        added: Vec::new(),
        removed: Vec::new(),
        changed: Vec::new(),
    };
    let requested_label = |requested: bool, extra: Option<bool>| match (requested, extra) {
        (true, _) => "requested".to_string(),
        (false, None) => "unrequested".to_string(),
        (false, Some(value)) => format!("unrequested {}", value as u8),
    };
    for port in snapshot {
        let installed: Vec<&Entry> = entries
            .iter()
            .filter(|entry| entry.name.eq_ignore_ascii_case(&port.port_name))
            .collect();
        if installed.is_empty() {
            result.removed.push(port.clone());
            continue;
        }
        let active = port.state == "installed";
        let requested = port.requested == 1;
        if active
            && let Some(current) = installed
                .iter()
                .find(|entry| !os_mismatch(entry, &os) && entry.active)
        {
            let mut changes = Vec::new();
            if port.requested_variants != current.requested_variants {
                changes.push((
                    "requested variants".into(),
                    port.requested_variants.clone(),
                    current.requested_variants.clone(),
                ));
            }
            if port.variants != current.variants {
                changes.push((
                    "variants".into(),
                    port.variants.clone(),
                    current.variants.clone(),
                ));
            }
            if requested != current.requested {
                changes.push((
                    "requested".into(),
                    requested_label(requested, None),
                    requested_label(current.requested, None),
                ));
            }
            if !changes.is_empty() {
                result.changed.push((port.clone(), changes));
            }
            continue;
        }
        let mut best: Option<(&Entry, i32)> = None;
        for entry in &installed {
            if os_mismatch(entry, &os) {
                continue;
            }
            let score = (entry.requested_variants == port.requested_variants) as i32
                + (entry.requested == requested) as i32
                + (entry.variants == port.variants) as i32
                + (entry.active == active) as i32;
            if best.is_none_or(|(_, best)| score > best) {
                best = Some((entry, score));
            }
        }
        let Some((entry, _)) = best else {
            result.removed.push(port.clone());
            continue;
        };
        let mut changes = Vec::new();
        if port.requested_variants != entry.requested_variants {
            changes.push((
                "requested variants".into(),
                port.requested_variants.clone(),
                entry.requested_variants.clone(),
            ));
        }
        if port.variants != entry.variants {
            changes.push((
                "variants".into(),
                port.variants.clone(),
                entry.variants.clone(),
            ));
        }
        if requested != entry.requested {
            changes.push((
                "requested".into(),
                requested_label(requested, Some(requested)),
                requested_label(entry.requested, Some(entry.requested)),
            ));
        }
        if active != entry.active {
            let state = |active: bool| if active { "installed" } else { "inactive" };
            changes.push((
                "state".into(),
                state(active).into(),
                state(entry.active).into(),
            ));
        }
        if !changes.is_empty() {
            result.changed.push((port.clone(), changes));
        }
    }
    for entry in entries {
        if os_mismatch(entry, &os)
            || snapshot
                .iter()
                .any(|port| port.port_name.eq_ignore_ascii_case(&entry.name))
        {
            continue;
        }
        result.added.push(SnapshotPort {
            port_name: entry.name.clone(),
            requested: entry.requested as i64,
            state: if entry.active { "installed" } else { "imaged" }.into(),
            variants: entry.variants.clone(),
            requested_variants: entry.requested_variants.clone(),
            port_files: Vec::new(),
        });
    }
    result
}

fn reader(path: &Path) -> Result<Registry> {
    Registry::open(Some(path))?.context("registry is missing")
}

fn find(path: &Path, id: i64) -> Result<Snapshot> {
    reader(path)?
        .snapshot(id)?
        .with_context(|| format!("Failed to obtain snapshot with ID {id}: no such snapshot"))
}

/// Create a snapshot of the active ports; inactive ones are listed and skipped.
pub(super) fn create(out: &mut impl Write, path: &Path, note: &str) -> Result<i64> {
    let entries = reader(path)?.entries()?;
    let mut inactive: Vec<String> = entries
        .iter()
        .filter(|entry| !entry.active)
        .map(|entry| {
            format!(
                "{} @{}_{} {}",
                entry.name, entry.version, entry.revision, entry.variants
            )
        })
        .collect();
    if !inactive.is_empty() {
        inactive.sort_by_key(|name| name.to_lowercase());
        writeln!(
            out,
            "The following inactive ports will not be a part of this snapshot and won't be installed while restoring:"
        )?;
        for port in inactive {
            writeln!(out, "{port}")?;
        }
    }
    let mut writer = Writer::open(path)?;
    let mut transaction = writer.transaction()?;
    let id = transaction.create_snapshot(note)?;
    transaction.commit()?;
    Ok(id)
}

pub(super) fn run(
    out: &mut impl Write,
    path: &Path,
    args: SnapshotArgs,
    quiet: bool,
) -> Result<()> {
    let operations = [
        args.list,
        args.create,
        args.diff.is_some(),
        args.delete.is_some(),
        args.export.is_some(),
        args.import.is_some(),
    ]
    .iter()
    .filter(|flag| **flag)
    .count();
    if operations > 1 {
        bail!(
            "Only one of the --list, --create, --diff, --delete, --export, or --import options can be specified.\nError: Incorrect usage, see port snapshot --help."
        );
    }
    if args.list {
        let snapshots = reader(path)?.snapshots()?;
        if snapshots.is_empty() {
            if !quiet {
                writeln!(
                    out,
                    "There are no snapshots. Use 'sudo port snapshot [--create] [--note '<message>']' to create one."
                )?;
            }
            return Ok(());
        }
        let times: Vec<String> = snapshots
            .iter()
            .map(|s| local_time(&s.created_at))
            .collect();
        let id = snapshots
            .iter()
            .map(|s| s.id.to_string().len())
            .max()
            .unwrap_or(0)
            .max(2);
        let created = times.iter().map(String::len).max().unwrap_or(0).max(7);
        let note = snapshots
            .iter()
            .map(|s| s.note.len())
            .max()
            .unwrap_or(0)
            .max(4);
        let heading = format!("{:>id$}  {:<created$}  {:<note$}", "ID", "Created", "Note");
        if !quiet {
            writeln!(out, "{heading}")?;
            writeln!(out, "{}", "=".repeat(heading.len()))?;
        }
        for (snapshot, time) in snapshots.iter().zip(&times) {
            writeln!(
                out,
                "{:>id$}  {time:<created$}  {:<note$}",
                snapshot.id, snapshot.note
            )?;
        }
        return Ok(());
    }
    if let Some(id) = args.diff {
        find(path, id)?;
        let registry = reader(path)?;
        let ports = registry.snapshot_ports(id, false)?;
        let mut diff = diff(&ports, &registry.entries()?);
        let mut note = String::new();
        if !args.all {
            note.push_str(
                "Showing differences in requested ports only. Re-run with --all to see all differences.\n",
            );
            diff.added.retain(|port| port.requested == 1);
            diff.removed.retain(|port| port.requested == 1);
            diff.changed.retain(|(port, _)| port.requested == 1);
        }
        // Upstream prints the requested variants only when there are none.
        let line = |port: &SnapshotPort| {
            if port.requested_variants.is_empty() {
                format!(" - {} \n", port.port_name)
            } else {
                format!(" - {}\n", port.port_name)
            }
        };
        let sort =
            |ports: &mut Vec<SnapshotPort>| ports.sort_by(|a, b| a.port_name.cmp(&b.port_name));
        sort(&mut diff.added);
        sort(&mut diff.removed);
        diff.changed
            .sort_by(|a, b| a.0.port_name.cmp(&b.0.port_name));
        if !diff.added.is_empty() {
            note.push_str("The following ports are installed but not in the snapshot:\n");
            diff.added
                .iter()
                .for_each(|port| note.push_str(&line(port)));
        }
        if !diff.removed.is_empty() {
            note.push_str("The following ports are in the snapshot but not installed:\n");
            diff.removed
                .iter()
                .for_each(|port| note.push_str(&line(port)));
        }
        if !diff.changed.is_empty() {
            note.push_str(
                "The following ports are in the snapshot and installed, but with changes:\n",
            );
            for (port, changes) in &diff.changed {
                note.push_str(&line(port));
                for (field, old, new) in changes {
                    note.push_str(&format!("   {field} changed from '{old}' to '{new}'\n"));
                }
            }
        }
        if diff.added.is_empty() && diff.removed.is_empty() && diff.changed.is_empty() {
            note.push_str("The current state and the specified snapshot match.\n");
        }
        writeln!(out, "{}", note.trim_end_matches('\n'))?;
        return Ok(());
    }
    if let Some(id) = args.delete {
        let mut writer = Writer::open(path)?;
        let mut transaction = writer.transaction()?;
        if !transaction.delete_snapshot(id)? {
            bail!("No such snapshot ID: {id}");
        }
        transaction.commit()?;
        return Ok(());
    }
    if let Some(id) = args.export {
        let snapshot = find(path, id)?;
        let ports = reader(path)?.snapshot_ports(id, true)?;
        let ports: Vec<serde_json::Value> = ports
            .into_iter()
            .map(|port| {
                serde_json::json!({
                    "port_name": port.port_name,
                    "requested": port.requested,
                    "state": port.state,
                    "variants": port.variants,
                    "requested_variants": port.requested_variants,
                    "port_files": port.port_files,
                })
            })
            .collect();
        let document = serde_json::json!({
            "metadata": {
                "type": "org.macports/snapshot/v1",
                "note": snapshot.note,
                "created_at": snapshot.created_at,
            },
            "ports": ports,
        });
        writeln!(out, "{}", serde_json::to_string_pretty(&document)?)?;
        return Ok(());
    }
    if let Some(file) = args.import {
        let contents =
            std::fs::read_to_string(&file).with_context(|| format!("Failed to read {file}"))?;
        let document: serde_json::Value =
            serde_json::from_str(&contents).context("Import failed: invalid JSON")?;
        let kind = document
            .pointer("/metadata/type")
            .and_then(serde_json::Value::as_str)
            .context("Import failed: Invalid format: no metadata/type field")?;
        if kind != "org.macports/snapshot/v1" {
            bail!(
                "Import failed: Invalid format: unsupported type {kind}, possibly generated by a newer version of MacPorts"
            );
        }
        let text = |pointer: &str| {
            document
                .pointer(pointer)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let ports: Vec<SnapshotPort> =
            serde_json::from_value(document.get("ports").cloned().unwrap_or_default())
                .context("Import failed: invalid ports")?;
        let mut writer = Writer::open(path)?;
        let mut transaction = writer.transaction()?;
        let id = transaction.import_snapshot(
            &text("/metadata/created_at"),
            &text("/metadata/note"),
            &ports,
        )?;
        transaction.commit()?;
        writeln!(out, "Snapshot successfully imported with ID {id}.")?;
        writeln!(
            out,
            "To restore this snapshot now, run\n\tsudo port restore --snapshot-id {id}"
        )?;
        return Ok(());
    }
    let note = args
        .note
        .map(|words| words.join(" "))
        .unwrap_or_else(|| "snapshot created for migration".into());
    create(out, path, &note)?;
    Ok(())
}
