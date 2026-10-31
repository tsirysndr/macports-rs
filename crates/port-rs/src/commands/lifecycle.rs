//! `port activate`, `deactivate` and `uninstall` for registered images.
use crate::{
    index::Port,
    install::{
        Action, Item, activation::Plan, archive::VerifiedArchive, deactivation, extract,
        inspect::Limits,
    },
    progress::Progress,
    registry::{self, Entry, Registry, dependents::Graph, saved, write::Writer},
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
    /// Do not run the port's Portfile hooks
    #[arg(long = "no-exec")]
    no_exec: bool,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}

#[derive(Args)]
pub struct Uninstall {
    /// Also uninstall the installed ports that depend on these ports
    #[arg(long = "follow-dependents")]
    follow_dependents: bool,
    /// Also uninstall dependencies that nothing else needs and nobody requested
    #[arg(long = "follow-dependencies")]
    follow_dependencies: bool,
    #[command(flatten)]
    lifecycle: Lifecycle,
}

/// Where Portfiles saved in the registry find the ports tree's resources.
pub(super) struct Hooks {
    pub root: Option<PathBuf>,
    pub quiet: bool,
}

impl Hooks {
    /// Run `target` from the Portfile saved with `entry`, so its pre- and
    /// post- hooks run around `body`. As upstream's `registry::run_target`,
    /// a missing or failing Portfile falls back to `body` alone.
    fn run(
        &self,
        path: &Path,
        entry: &Entry,
        target: &str,
        no_exec: bool,
        progress: &Progress,
        body: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        let opened = match (&self.root, no_exec) {
            (Some(root), false) => self.open(path, root, entry).unwrap_or_else(|err| {
                progress.log(
                    "debug",
                    &format!(
                        "cannot open the registry Portfile of {}: {err:#}",
                        label(entry)
                    ),
                );
                None
            }),
            _ => None,
        };
        let Some((host, mut session)) = opened else {
            return body();
        };
        let mut body = Some(body);
        let mut done = false;
        let result = (|| -> Result<()> {
            let worker = crate::install::mirror_worker::MirrorWorker::new(
                host.context.mirror_timings.clone(),
                1,
            )?;
            let label = super::phase::prepare(&mut session, &host, &worker, &entry.name)?;
            label.enter(&mut session, target)?;
            let item = session.scalar(&format!("org.macports.{target}"))?;
            session.run_prepared_target_with_native_body(
                &item,
                false,
                &format!("port{target}::{target}_main"),
                |_| {
                    let result = body.take().context("hook body already ran")?();
                    done = result.is_ok();
                    result
                },
            )?;
            Ok(())
        })();
        host.progress.close_log();
        match result {
            Ok(()) => Ok(()),
            Err(err) => {
                progress.warning(&format!(
                    "Failed to execute portfile from registry for {}",
                    label(entry)
                ));
                progress.log("debug", &format!("{err:#}"));
                match body.take() {
                    Some(body) => body(),
                    None if done => Ok(()),
                    None => Err(err),
                }
            }
        }
    }

    fn open(
        &self,
        path: &Path,
        root: &Path,
        entry: &Entry,
    ) -> Result<Option<(super::local::Host, crate::portfile::Session)>> {
        let Some(registry) = Registry::open(Some(path))? else {
            return Ok(None);
        };
        let Some(key) = registry.portfile(entry)? else {
            return Ok(None);
        };
        let directory = saved::registry_directory(path);
        let portdir =
            saved::portfile_directory(directory, &entry.name, &entry.version, entry.revision, &key);
        if !portdir.join("Portfile").is_file() {
            return Ok(None);
        }
        let groups = registry
            .portgroups(entry)?
            .iter()
            .map(|group| group.directory(directory))
            .collect();
        drop(registry);
        crate::index::allow_external_directory_with_groups(&portdir, groups);
        let port = Port {
            name: entry.name.clone(),
            fields: BTreeMap::from([
                ("portdir".into(), portdir.to_string_lossy().into_owned()),
                ("version".into(), entry.version.clone()),
                ("revision".into(), entry.revision.to_string()),
                ("epoch".into(), entry.epoch.clone()),
            ]),
        };
        // Negative variants come from the request, positive ones from the image.
        let mut spec = crate::spec::PortSpec::named(&entry.name);
        if !entry.requested_variants.is_empty() {
            spec.parse_modifier(&entry.requested_variants)?;
            spec.variants.retain(|_, enabled| !*enabled);
        }
        if !entry.variants.is_empty() {
            spec.parse_modifier(&entry.variants)?;
        }
        let host = super::local::Host::new(root, Some(path), self.quiet)?;
        let session = host.open(root, &port, &spec)?;
        Ok(Some((host, session)))
    }
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

pub(super) fn deactivate(
    path: &Path,
    args: Lifecycle,
    hooks: &Hooks,
    progress: &Progress,
) -> Result<()> {
    for spec in registry::expanded_specifications(&args.ports)? {
        let snapshot = snapshot(path)?;
        let matches = matching(&snapshot.entries, &spec);
        let Some(active) = matches.iter().find(|entry| entry.active) else {
            if matches.is_empty() {
                bail!("Image error: port {} is not installed", spec.name);
            }
            bail!("Image error: port {} is not active", spec.name);
        };
        check_dependents(
            &snapshot,
            active,
            true,
            args.force || crate::options::get().force,
            "deactivate",
            progress,
        )?;
        hooks.run(path, active, "deactivate", args.no_exec, progress, || {
            progress.phase(&format!("Deactivating {}", label(active)));
            let mut writer = open_writer(path)?;
            deactivation::deactivate(&mut writer, active.id, &snapshot.files[&active.id])
        })?;
    }
    Ok(())
}

/// Deactivate one active image without the dependents check.
pub(super) fn deactivate_entry(path: &Path, entry: &Entry, progress: &Progress) -> Result<()> {
    let snapshot = snapshot(path)?;
    progress.phase(&format!("Deactivating {}", label(entry)));
    let mut writer = open_writer(path)?;
    deactivation::deactivate(&mut writer, entry.id, &snapshot.files[&entry.id])
}

pub(super) fn uninstall(
    path: &Path,
    args: Uninstall,
    hooks: &Hooks,
    progress: &Progress,
) -> Result<()> {
    for spec in registry::expanded_specifications(&args.lifecycle.ports)? {
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
        let matches: Vec<Entry> = matches.into_iter().cloned().collect();
        for entry in &matches {
            uninstall_one(
                path,
                entry,
                &args,
                args.follow_dependencies,
                hooks,
                progress,
            )?;
        }
    }
    Ok(())
}

fn uninstall_one(
    path: &Path,
    entry: &Entry,
    args: &Uninstall,
    follow_dependencies: bool,
    hooks: &Hooks,
    progress: &Progress,
) -> Result<()> {
    let current = snapshot(path)?;
    // A dependent or dependency removed earlier may have taken this one along.
    let Some(entry) = current.entries.iter().find(|known| known.id == entry.id) else {
        return Ok(());
    };
    if args.follow_dependents {
        let registry = Registry::open(Some(path))?.context("no ports are installed")?;
        let dependents: Vec<Entry> = Graph::build(&registry, &current.entries)?
            .direct(&entry.name)
            .cloned()
            .collect();
        drop(registry);
        for dependent in &dependents {
            uninstall_one(path, dependent, args, false, hooks, progress)?;
        }
    } else {
        check_dependents(
            &current,
            entry,
            false,
            args.lifecycle.force || crate::options::get().force,
            "uninstall",
            progress,
        )?;
    }
    let dependencies = if follow_dependencies {
        dependency_names(path, entry.id)?
    } else {
        Vec::new()
    };
    let current = snapshot(path)?;
    let no_exec = args.lifecycle.no_exec;
    hooks.run(path, entry, "uninstall", no_exec, progress, || {
        remove_image(path, &current, entry, hooks, no_exec, progress)
    })?;
    if follow_dependencies {
        let unneeded = unneeded_dependencies(path, dependencies)?;
        if !unneeded.is_empty() {
            progress.message("The following dependencies will be uninstalled:");
            for dependency in &unneeded {
                progress.message(&format!("  {}", label(dependency)));
            }
            if !super::ask("Continue?", true) {
                return Ok(());
            }
            for dependency in &unneeded {
                uninstall_one(path, dependency, args, false, hooks, progress)?;
            }
        }
    }
    Ok(())
}

fn dependency_names(path: &Path, id: i64) -> Result<Vec<String>> {
    let Some(registry) = Registry::open(Some(path))? else {
        return Ok(Vec::new());
    };
    let mut names: Vec<String> = registry
        .dependent_edges()?
        .into_iter()
        .filter(|(_, dependent)| *dependent == id)
        .map(|(name, _)| name)
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// Unrequested images among `candidates` (and, transitively, their own
/// dependencies) whose dependents are all being uninstalled too, ordered so
/// dependents go first.
fn unneeded_dependencies(path: &Path, mut candidates: Vec<String>) -> Result<Vec<Entry>> {
    let Some(registry) = Registry::open(Some(path))? else {
        return Ok(Vec::new());
    };
    let entries = registry.entries()?;
    let graph = Graph::build(&registry, &entries)?;
    let edges = registry.dependent_edges()?;
    let mut planned: Vec<Entry> = Vec::new();
    let mut position = 0;
    while position < candidates.len() {
        let name = candidates[position].clone();
        position += 1;
        for image in entries
            .iter()
            .filter(|image| image.name.eq_ignore_ascii_case(&name) && !image.requested)
        {
            if planned.iter().any(|known| known.id == image.id) {
                continue;
            }
            let all_leaving = graph
                .direct(&image.name)
                .all(|dependent| planned.iter().any(|known| known.id == dependent.id));
            if !all_leaving {
                continue;
            }
            planned.push(image.clone());
            for (dependency, _) in edges.iter().filter(|(_, id)| *id == image.id) {
                if !candidates[position..].contains(dependency) {
                    candidates.push(dependency.clone());
                }
            }
        }
    }
    Ok(planned)
}

/// Uninstall one registered image (deactivating it first), without the
/// dependent check, for callers such as `upgrade -u`.
pub(super) fn uninstall_entry(path: &Path, entry: &Entry, progress: &Progress) -> Result<()> {
    let snapshot = snapshot(path)?;
    let hooks = Hooks {
        root: None,
        quiet: true,
    };
    remove_image(path, &snapshot, entry, &hooks, true, progress)
}

fn remove_image(
    path: &Path,
    snapshot: &Snapshot,
    entry: &Entry,
    hooks: &Hooks,
    no_exec: bool,
    progress: &Progress,
) -> Result<()> {
    if entry.active {
        hooks.run(path, entry, "deactivate", no_exec, progress, || {
            progress.phase(&format!("Deactivating {}", label(entry)));
            let mut writer = open_writer(path)?;
            deactivation::deactivate(&mut writer, entry.id, &snapshot.files[&entry.id])
        })?;
    }
    let (portfile, groups) = match Registry::open(Some(path))? {
        Some(registry) => (registry.portfile(entry)?, registry.portgroups(entry)?),
        None => (None, Vec::new()),
    };
    progress.phase(&format!("Uninstalling {}", label(entry)));
    let mut writer = open_writer(path)?;
    let mut transaction = writer.transaction()?;
    transaction.remove(entry.id)?;
    transaction.commit()?;
    drop(writer);
    // Saved Portfiles and PortGroups go once no other image uses them.
    if let Some(registry) = Registry::open(Some(path))? {
        let directory = saved::registry_directory(path);
        if let Some(key) = portfile
            && registry.portfile_users(&entry.name, &entry.version, entry.revision, &key)? == 0
        {
            saved::remove_portfile(directory, &entry.name, &entry.version, entry.revision, &key);
        }
        for group in &groups {
            if registry.portgroup_users(group)? == 0 {
                saved::remove_group(directory, group);
            }
        }
    }
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
    hooks: &Hooks,
    progress: &Progress,
    staging: &Path,
) -> Result<()> {
    for spec in registry::expanded_specifications(&args.ports)? {
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
        hooks.run(path, target, "activate", args.no_exec, progress, || {
            activate_entry(path, target, progress, staging)
        })?;
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
