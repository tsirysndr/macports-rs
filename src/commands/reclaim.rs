//! `port reclaim` (macports::reclaim): uninstall unrequested ports nothing
//! requested needs and inactive images, then delete unused distfiles, build
//! directories and the ccache.
use crate::{
    index::Index,
    progress::Progress,
    query,
    registry::{Entry, Registry},
    spec::PortSpec,
};
use anyhow::{Context, Result};
use clap::Args;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufRead, IsTerminal, Write},
    path::{Path, PathBuf},
};

#[derive(Args)]
pub struct Reclaim {
    /// Keep build dependencies of requested ports
    #[arg(long = "keep-build-deps")]
    keep_build_deps: bool,
    /// Remind every two weeks to run reclaim
    #[arg(long = "enable-reminders")]
    enable_reminders: bool,
    /// Stop reminding to run reclaim
    #[arg(long = "disable-reminders")]
    disable_reminders: bool,
    #[arg(skip)]
    pub dry_run: bool,
}

/// Registry entries with their (dependency name, dependent id) edges.
type Installed = (Vec<Entry>, Vec<(String, i64)>);

const BUILD_FIELDS: [&str; 4] = [
    "depends_fetch",
    "depends_extract",
    "depends_patch",
    "depends_build",
];

/// Ask on a terminal; non-interactive runs take the default answer, as upstream.
fn confirm(question: &str, items: &[String]) -> bool {
    if crate::options::get().noninteractive || !std::io::stdin().is_terminal() {
        return true;
    }
    for item in items {
        eprintln!("  {item}");
    }
    eprint!("{question} [Y/n]: ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    !matches!(answer.trim(), "n" | "N" | "no" | "No")
}

fn label(entry: &Entry) -> String {
    format!("{} @{}{}", entry.name, entry.full_version(), entry.variants)
}

/// Dependents before their dependencies, so uninstalls never orphan anything.
fn dependents_first(entries: Vec<Entry>, edges: &[(String, i64)]) -> Vec<Entry> {
    let mut remaining = entries;
    let mut ordered = Vec::new();
    while !remaining.is_empty() {
        let names: BTreeSet<String> = remaining
            .iter()
            .map(|entry| entry.name.to_ascii_lowercase())
            .collect();
        let ids: BTreeSet<i64> = remaining.iter().map(|entry| entry.id).collect();
        let needed: BTreeSet<String> = edges
            .iter()
            .filter(|(_, id)| ids.contains(id))
            .map(|(name, _)| name.to_ascii_lowercase())
            .filter(|name| names.contains(name))
            .collect();
        let (leaves, rest): (Vec<Entry>, Vec<Entry>) = remaining
            .into_iter()
            .partition(|entry| !needed.contains(&entry.name.to_ascii_lowercase()));
        if leaves.is_empty() {
            ordered.extend(rest);
            break;
        }
        ordered.extend(leaves);
        remaining = rest;
    }
    ordered
}

fn variants(entry: &Entry) -> String {
    if entry.requested_variants.is_empty() {
        entry.variants.clone()
    } else {
        entry.requested_variants.clone()
    }
}

fn spec_for(entry: &Entry) -> Result<PortSpec> {
    let mut spec = PortSpec::named(&entry.name);
    let text = variants(entry);
    let mut rest = text.as_str();
    while let Some(start) = rest.find(['+', '-']) {
        let tail = &rest[start + 1..];
        let end = tail.find(['+', '-']).unwrap_or(tail.len());
        spec.parse_modifier(&rest[start..start + 1 + end])?;
        rest = &tail[end..];
    }
    Ok(spec)
}

struct Ports<'a> {
    host: super::local::Host,
    index: &'a Index,
    root: &'a Path,
}

impl Ports<'_> {
    fn build_dependencies(&self, entry: &Entry) -> Vec<String> {
        let Ok(port) = query::lookup(self.index, &entry.name) else {
            return Vec::new();
        };
        let Ok(spec) = spec_for(entry) else {
            return Vec::new();
        };
        let Ok(mut session) = self.host.open(self.root, port, &spec) else {
            return Vec::new();
        };
        BUILD_FIELDS
            .iter()
            .filter_map(|field| session.scalar(field).ok())
            .flat_map(|value| crate::tcl_list::parse(&value).unwrap_or_default())
            .filter_map(|dependency| dependency.rsplit(':').next().map(str::to_string))
            .collect()
    }

    /// `(dist_subdir, distfile)` pairs still needed by installed images.
    fn distfiles(&self, entry: &Entry, progress: &Progress) -> Vec<(String, String)> {
        let open = || -> Result<Vec<(String, String)>> {
            let port = query::lookup(self.index, &entry.name)?;
            let mut session = self.host.open(self.root, port, &spec_for(entry)?)?;
            let subdir = session.scalar("dist_subdir")?;
            let filespath = PathBuf::from(session.scalar("filespath")?);
            let mut files = Vec::new();
            for name in ["distfiles", "patchfiles"] {
                let Ok(value) = session.scalar(name) else {
                    continue;
                };
                for file in crate::tcl_list::parse(&value).unwrap_or_default() {
                    let distfile = session.invoke("getdistname", &[&file])?;
                    if !filespath.join(&distfile).exists() {
                        files.push((subdir.clone(), distfile));
                    }
                }
            }
            Ok(files)
        };
        open().unwrap_or_else(|err| {
            progress.warning(&format!(
                "Failed to open port {} {}: {err:#}",
                entry.name,
                variants(entry)
            ));
            Vec::new()
        })
    }
}

fn walk(directory: &Path, keep: &BTreeSet<PathBuf>, unused: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => walk(&path, keep, unused),
            Ok(_) if !keep.contains(&path) => unused.push(path),
            _ => {}
        }
    }
}

fn delete_children(directory: &Path, progress: &Progress) {
    for entry in fs::read_dir(directory).into_iter().flatten().flatten() {
        let path = entry.path();
        let result = if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        if let Err(err) = result {
            progress.error(&format!("cannot delete {}: {err}", path.display()));
        }
    }
}

fn uninstall(
    registry_path: &Path,
    entries: Vec<Entry>,
    edges: &[(String, i64)],
    dry_run: bool,
    question: &str,
    progress: &Progress,
) -> Result<()> {
    let entries = dependents_first(entries, edges);
    let names: Vec<String> = entries.iter().map(label).collect();
    if !confirm(question, &names) {
        return Ok(());
    }
    if dry_run {
        return Ok(());
    }
    for entry in &entries {
        if let Err(err) = super::lifecycle::uninstall_entry(registry_path, entry, progress) {
            progress.error(&format!("Error uninstalling {}: {err:#}", entry.name));
        }
    }
    Ok(())
}

/// Delete registry snapshots older than 30 days.
fn remove_snapshots(registry_path: &Path, progress: &Progress) -> Result<()> {
    let Some(registry) = Registry::open(Some(registry_path))? else {
        return Ok(());
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_secs() as i64)
        .unwrap_or(0);
    let old: Vec<_> = registry
        .snapshots()?
        .into_iter()
        .filter(|snapshot| {
            super::snapshot::utc_seconds(&snapshot.created_at)
                .is_some_and(|created| now - created > 2_592_000)
        })
        .collect();
    drop(registry);
    if old.is_empty() {
        progress.log("info", "No registry snapshots older than 30 days to delete");
        return Ok(());
    }
    let listing: Vec<String> = old
        .iter()
        .map(|snapshot| {
            format!(
                "{} {}",
                super::snapshot::local_time(&snapshot.created_at),
                snapshot.note
            )
        })
        .collect();
    progress.message("Registry snapshots older than 30 days found:");
    if confirm("Would you like to delete them?", &listing) {
        let mut writer = crate::registry::write::Writer::open(registry_path)?;
        let mut transaction = writer.transaction()?;
        for snapshot in &old {
            transaction.delete_snapshot(snapshot.id)?;
        }
        transaction.commit()?;
    }
    Ok(())
}

pub(super) fn run(
    index: &Index,
    registry_path: &Path,
    args: Reclaim,
    quiet: bool,
    root: &Path,
) -> Result<()> {
    let host = super::local::Host::new(root, Some(registry_path), quiet)?;
    let progress = host.progress.clone();
    let variables = host.context.variables.clone();
    let variable = |name: &str| variables.get(name).cloned().unwrap_or_default();
    let portdbpath = PathBuf::from(variable("portdbpath"));
    let last_run = portdbpath.join("last_reclaim");
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|time| time.as_secs())
            .unwrap_or(0)
            .to_string()
    };
    if args.enable_reminders {
        progress.log("info", "Enabling port reclaim reminders.");
        fs::write(&last_run, format!("{}\n", now()))?;
        return Ok(());
    }
    if args.disable_reminders {
        progress.log("info", "Disabling port reclaim reminders.");
        fs::write(&last_run, "disabled\n")?;
        return Ok(());
    }
    let context = Ports { host, index, root };
    let dry = if args.dry_run { " (dry run)" } else { "" };
    let read = || -> Result<Installed> {
        match Registry::open(Some(registry_path))? {
            Some(registry) => Ok((registry.entries()?, registry.dependent_edges()?)),
            None => Ok((Vec::new(), Vec::new())),
        }
    };

    progress.phase("Checking for unnecessary unrequested ports");
    let (entries, edges) = read()?;
    let installed: Vec<&Entry> = entries.iter().filter(|entry| entry.active).collect();
    let by_name: BTreeMap<String, &Entry> = installed
        .iter()
        .map(|entry| (entry.name.to_ascii_lowercase(), *entry))
        .collect();
    let mut dependencies: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    for (name, id) in &edges {
        dependencies
            .entry(*id)
            .or_default()
            .push(name.to_ascii_lowercase());
    }
    let mut needed = BTreeSet::new();
    let mut stack: Vec<&Entry> = installed.iter().copied().filter(|e| e.requested).collect();
    while let Some(entry) = stack.pop() {
        if !needed.insert(entry.name.to_ascii_lowercase()) {
            continue;
        }
        let mut names = dependencies.get(&entry.id).cloned().unwrap_or_default();
        if args.keep_build_deps {
            names.extend(
                context
                    .build_dependencies(entry)
                    .into_iter()
                    .map(|name| name.to_ascii_lowercase()),
            );
        }
        stack.extend(names.iter().filter_map(|name| by_name.get(name).copied()));
    }
    let unnecessary: Vec<Entry> = installed
        .iter()
        .filter(|entry| !needed.contains(&entry.name.to_ascii_lowercase()))
        .map(|entry| (*entry).clone())
        .collect();
    if unnecessary.is_empty() {
        progress.message("Found no unrequested ports without requested dependents.");
    } else {
        progress.message("Unrequested ports without requested dependents found:");
        uninstall(
            registry_path,
            unnecessary,
            &edges,
            args.dry_run,
            "Would you like to uninstall them?",
            &progress,
        )?;
        if args.dry_run {
            progress.message("Skipping uninstall of unrequested ports (dry run)");
        }
    }

    progress.phase("Checking for inactive ports");
    let (entries, edges) = read()?;
    let inactive: Vec<Entry> = entries.into_iter().filter(|entry| !entry.active).collect();
    if inactive.is_empty() {
        progress.message("Found no inactive ports.");
    } else {
        progress.message("Inactive ports found:");
        uninstall(
            registry_path,
            inactive,
            &edges,
            args.dry_run,
            "Would you like to uninstall them?",
            &progress,
        )?;
        if args.dry_run {
            progress.message("Skipping uninstall of inactive ports (dry run)");
        }
    }

    progress.phase("Building list of distfiles still in use");
    let (entries, _) = read()?;
    let distfiles = portdbpath.join("distfiles");
    let home = std::env::var_os("HOME").map(PathBuf::from).map(|home| {
        home.join(".macports")
            .join(distfiles.strip_prefix("/").unwrap_or(&distfiles))
    });
    let mut keep = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for entry in &entries {
        if !seen.insert((entry.name.clone(), variants(entry))) {
            continue;
        }
        for (subdir, file) in context.distfiles(entry, &progress) {
            for base in std::iter::once(&distfiles).chain(home.as_ref()) {
                let path = base.join(&subdir).join(&file);
                if path.is_file() {
                    progress.log("info", &format!("Keeping {}", path.display()));
                    keep.insert(path);
                }
            }
        }
    }
    progress.phase("Searching for unused distfiles");
    let mut unused = Vec::new();
    walk(&distfiles, &keep, &mut unused);
    if let Some(home) = &home {
        walk(home, &keep, &mut unused);
    }
    if unused.is_empty() {
        progress.message("No unused files found.");
    } else {
        let size: u64 = unused
            .iter()
            .filter_map(|path| fs::metadata(path).ok())
            .map(|metadata| metadata.len())
            .sum();
        let listing: Vec<String> = unused
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        progress.message(&format!(
            "Found {} files (total {}) that are no longer needed and can be deleted.",
            unused.len(),
            super::size::bytesize(size, None)
        ));
        if confirm("Delete them?", &listing) {
            progress.message(&format!("Deleting...{dry}"));
            if !args.dry_run {
                for path in &unused {
                    if let Err(err) = fs::remove_file(path) {
                        progress.warning(&format!("Could not delete {}: {err}", path.display()));
                        continue;
                    }
                    let mut directory = path.parent();
                    while let Some(current) = directory {
                        let inside = current.starts_with(&distfiles) && current != distfiles
                            || home
                                .as_ref()
                                .is_some_and(|home| current.starts_with(home) && current != home);
                        if !inside || fs::remove_dir(current).is_err() {
                            break;
                        }
                        directory = current.parent();
                    }
                }
            }
        } else {
            progress.message("OK, keeping the files.");
        }
    }

    let build = portdbpath.join("build");
    progress.phase(&format!("Build location: {}", build.display()));
    if args.dry_run {
        progress.message("Deleting... (dry run)");
    } else if fs::read_dir(&build).is_ok_and(|mut entries| entries.next().is_some()) {
        if confirm("Would you like to delete all the build directories?", &[]) {
            delete_children(&build, &progress);
        }
    } else {
        progress.log("info", "No build directories to delete");
    }

    let ccache = PathBuf::from(variable("ccache_dir"));
    if ccache.as_os_str().is_empty() || !ccache.exists() {
        progress.log(
            "info",
            &format!(
                "Skipping deletion of ccache directory: {} does not exist.",
                ccache.display()
            ),
        );
    } else {
        progress.phase(&format!("ccache location: {}", ccache.display()));
        if args.dry_run {
            progress.message("Deleting... (dry run)");
        } else if confirm(
            "Would you like to delete everything under the ccache directory?",
            &[],
        ) {
            delete_children(&ccache, &progress);
        }
    }

    if args.dry_run {
        progress.message("Deleting snapshots... (dry run)");
    } else {
        remove_snapshots(registry_path, &progress)?;
    }

    if !args.dry_run {
        let contents = fs::read_to_string(&last_run).unwrap_or_default();
        if contents.trim() != "disabled" {
            fs::write(&last_run, format!("{}\n", now()))
                .with_context(|| format!("cannot write {}", last_run.display()))?;
        }
    }
    Ok(())
}
