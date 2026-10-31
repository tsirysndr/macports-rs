//! `port activate`, `deactivate` and `uninstall` for registered images.
use crate::{
    index::Port,
    install::{
        Action, Item, activation::Plan, archive::VerifiedArchive, deactivation, extract,
        inspect::Limits,
    },
    progress::Progress,
    registry::{self, Entry, Registry, dependents::Graph, write::Writer},
};
use anyhow::{Context, Result, bail, ensure};
use clap::Args;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Args)]
pub struct Lifecycle {
    /// Proceed even if other ports depend on the image
    #[arg(short = 'f', long = "force")]
    force: bool,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}

struct Snapshot {
    entries: Vec<Entry>,
    /// Names of each image's dependents, with their activity.
    dependents: BTreeMap<i64, Vec<(String, bool)>>,
    files: BTreeMap<i64, Vec<String>>,
    locations: BTreeMap<i64, String>,
}

fn snapshot(path: &Path) -> Result<Snapshot> {
    let registry = Registry::open(Some(path))?.context("no ports are installed")?;
    let entries = registry.entries()?;
    let graph = Graph::build(&registry, &entries)?;
    let mut dependents = BTreeMap::new();
    let mut files = BTreeMap::new();
    let mut locations = BTreeMap::new();
    for entry in &entries {
        dependents.insert(
            entry.id,
            graph
                .direct(&entry.name)
                .map(|dependent| {
                    (
                        format!("{} @{}", dependent.name, dependent.full_version()),
                        dependent.active,
                    )
                })
                .collect(),
        );
        files.insert(entry.id, registry.files(entry)?);
        locations.insert(entry.id, registry.location(entry)?);
    }
    Ok(Snapshot {
        entries,
        dependents,
        files,
        locations,
    })
}

fn matching<'a>(entries: &'a [Entry], spec: &crate::spec::PortSpec) -> Vec<&'a Entry> {
    let version = spec.composite_version();
    entries
        .iter()
        .filter(|entry| {
            entry.name.eq_ignore_ascii_case(&spec.name)
                && (version.is_empty()
                    || version == entry.version
                    || version == entry.full_version())
        })
        .collect()
}

fn label(entry: &Entry) -> String {
    format!("{} @{}", entry.name, entry.full_version())
}

fn check_dependents(
    snapshot: &Snapshot,
    entry: &Entry,
    active_only: bool,
    force: bool,
    action: &str,
    progress: &Progress,
) -> Result<()> {
    let blocking: Vec<&String> = snapshot.dependents[&entry.id]
        .iter()
        .filter(|(_, active)| *active || !active_only)
        .map(|(name, _)| name)
        .collect();
    if blocking.is_empty() {
        return Ok(());
    }
    let list = blocking
        .iter()
        .map(|name| format!("  {name}"))
        .collect::<Vec<_>>()
        .join("\n");
    if force {
        progress.warning(&format!(
            "{} is needed by the following ports, but {action} was forced:\n{list}",
            label(entry)
        ));
        return Ok(());
    }
    bail!(
        "Unable to {action} {}, the following ports depend on it:\n{list}",
        label(entry)
    )
}

fn open_writer(path: &Path) -> Result<Writer> {
    Writer::open(path).with_context(|| format!("cannot open registry {}", path.display()))
}

pub(super) fn deactivate(path: &Path, args: Lifecycle, progress: &Progress) -> Result<()> {
    for spec in registry::specifications(&args.ports)? {
        let snapshot = snapshot(path)?;
        let matches = matching(&snapshot.entries, &spec);
        let Some(active) = matches.iter().find(|entry| entry.active) else {
            if matches.is_empty() {
                bail!("Image error: port {} is not installed", spec.name);
            }
            bail!("Image error: port {} is not active", spec.name);
        };
        check_dependents(&snapshot, active, true, args.force, "deactivate", progress)?;
        progress.phase(&format!("Deactivating {}", label(active)));
        let mut writer = open_writer(path)?;
        deactivation::deactivate(&mut writer, active.id, &snapshot.files[&active.id])?;
    }
    Ok(())
}

pub(super) fn uninstall(path: &Path, args: Lifecycle, progress: &Progress) -> Result<()> {
    for spec in registry::specifications(&args.ports)? {
        let snapshot = snapshot(path)?;
        let matches = matching(&snapshot.entries, &spec);
        ensure!(
            !matches.is_empty(),
            "port {} not registered as installed",
            spec.name
        );
        if spec.composite_version().is_empty() && matches.len() > 1 {
            bail!(
                "The following versions of {} are currently installed; specify one:\n{}",
                spec.name,
                matches
                    .iter()
                    .map(|entry| format!(
                        "  {}{}",
                        label(entry),
                        if entry.active { " (active)" } else { "" }
                    ))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
        for entry in matches {
            check_dependents(&snapshot, entry, false, args.force, "uninstall", progress)?;
            remove_image(path, &snapshot, entry, progress)?;
        }
    }
    Ok(())
}

/// Uninstall one registered image (deactivating it first), without the
/// dependent check, for callers such as `upgrade -u`.
pub(super) fn uninstall_entry(path: &Path, entry: &Entry, progress: &Progress) -> Result<()> {
    let snapshot = snapshot(path)?;
    remove_image(path, &snapshot, entry, progress)
}

fn remove_image(
    path: &Path,
    snapshot: &Snapshot,
    entry: &Entry,
    progress: &Progress,
) -> Result<()> {
    let mut writer = open_writer(path)?;
    if entry.active {
        progress.phase(&format!("Deactivating {}", label(entry)));
        deactivation::deactivate(&mut writer, entry.id, &snapshot.files[&entry.id])?;
    }
    progress.phase(&format!("Uninstalling {}", label(entry)));
    let mut transaction = writer.transaction()?;
    transaction.remove(entry.id)?;
    transaction.commit()?;
    let location = PathBuf::from(&snapshot.locations[&entry.id]);
    for file in [
        location.clone(),
        PathBuf::from(format!("{}.sig", location.display())),
    ] {
        match fs::remove_file(&file) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => progress.warning(&format!("cannot remove {}: {err}", file.display())),
        }
    }
    if let Some(directory) = location.parent()
        && fs::read_dir(directory).is_ok_and(|mut entries| entries.next().is_none())
    {
        let _ = fs::remove_dir(directory);
    }
    Ok(())
}

pub(super) fn activate(
    path: &Path,
    args: Lifecycle,
    progress: &Progress,
    staging: &Path,
) -> Result<()> {
    for spec in registry::specifications(&args.ports)? {
        let snapshot = snapshot(path)?;
        let matches = matching(&snapshot.entries, &spec);
        ensure!(
            !matches.is_empty(),
            "Image error: port {} not registered as installed",
            spec.name
        );
        let inactive: Vec<&&Entry> = matches.iter().filter(|entry| !entry.active).collect();
        let Some(&&target) = inactive.last() else {
            progress.message(&format!("port {} is already active", spec.name));
            continue;
        };
        ensure!(
            inactive.len() == 1 || !spec.composite_version().is_empty(),
            "multiple inactive versions of {} are installed; specify one",
            spec.name
        );
        activate_entry(path, target, progress, staging)?;
    }
    Ok(())
}

/// Activate an installed image, deactivating another active version first.
pub(super) fn activate_entry(
    path: &Path,
    target: &Entry,
    progress: &Progress,
    staging: &Path,
) -> Result<()> {
    let snapshot = snapshot(path)?;
    if let Some(active) = snapshot
        .entries
        .iter()
        .find(|entry| entry.active && entry.name.eq_ignore_ascii_case(&target.name))
    {
        progress.phase(&format!("Deactivating {}", label(active)));
        let mut writer = open_writer(path)?;
        deactivation::deactivate(&mut writer, active.id, &snapshot.files[&active.id])?;
    }
    progress.phase(&format!("Activating {}", label(target)));
    let location = &snapshot.locations[&target.id];
    let signature = PathBuf::from(format!("{location}.sig"));
    ensure!(
        !signature.exists(),
        "re-activating signed binary images is not supported yet; reinstall {}",
        target.name
    );
    let mut archive = VerifiedArchive::local(
        fs::File::open(location).with_context(|| format!("cannot open image {location}"))?,
    )?;
    let port = Port {
        name: target.name.clone(),
        fields: BTreeMap::from([
            ("version".into(), target.version.clone()),
            ("revision".into(), target.revision.to_string()),
            ("epoch".into(), target.epoch.clone()),
        ]),
    };
    let item = Item {
        port: &port,
        variants: target.variants.clone(),
        variants_resolved: true,
        provider_fallbacks: Vec::new(),
        requested: target.requested,
        action: Action::Activate,
    };
    let prepared = extract::prepare(&mut archive, &item, staging, Limits::default())?;
    let mut writer = open_writer(path)?;
    Plan::inspect(&prepared, Path::new("/"))?.activate(&mut writer, target.id)?;
    Ok(())
}
