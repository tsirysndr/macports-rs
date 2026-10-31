//! `port rev-upgrade` (macports::revupgrade): find active binaries with broken
//! dylib linkage and rebuild the ports that own them.
use crate::{
    index::Index,
    macho::{self, LC_LOAD_WEAK_DYLIB, Slice},
    progress::Progress,
    registry::{Entry, Registry},
    spec::PortSpec,
};
use anyhow::{Result, bail};
use clap::Args;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::Path,
};

#[derive(Args)]
pub struct RevUpgrade {
    /// Also check that each library's install name refers to itself
    #[arg(long = "id-loadcmd-check")]
    id_loadcmd_check: bool,
    #[arg(skip)]
    pub dry_run: bool,
}

unsafe extern "C" {
    fn _dyld_shared_cache_contains_path(path: *const libc::c_char) -> bool;
}

/// Libraries that exist only in the dyld shared cache have no file on disk.
fn in_shared_cache(path: &str) -> bool {
    let Ok(path) = std::ffi::CString::new(path) else {
        return false;
    };
    // SAFETY: dyld only reads the NUL-terminated path.
    unsafe { _dyld_shared_cache_contains_path(path.as_ptr()) }
}

enum Library {
    Slices(Vec<Slice>),
    Cached,
    NotMachO,
    Missing(String),
}

fn library(path: &str, cache: &mut BTreeMap<String, std::rc::Rc<Library>>) -> std::rc::Rc<Library> {
    cache
        .entry(path.to_string())
        .or_insert_with(|| {
            std::rc::Rc::new(match fs::File::open(path) {
                Ok(mut file) => match macho::slices(&mut file) {
                    Ok(Some(slices)) => Library::Slices(slices),
                    _ => Library::NotMachO,
                },
                Err(_) if in_shared_cache(path) => Library::Cached,
                Err(err) => Library::Missing(err.to_string()),
            })
        })
        .clone()
}

/// `@loader_path` resolves against the binary; `@executable_path` and
/// `@rpath` cannot be resolved statically and are skipped.
fn special_path(binary: &str, path: &str) -> Option<String> {
    if path.contains("@executable_path") || path.contains("@rpath") {
        return None;
    }
    let directory = Path::new(binary).parent().unwrap_or(Path::new("/"));
    Some(path.replacen("@loader_path", &directory.to_string_lossy(), 1))
}

fn runnable(architecture: &str, darwin: u32, host: &str) -> bool {
    !(darwin >= 11 && architecture.starts_with("ppc")
        || host == "i386" && matches!(architecture, "arm64" | "ppc64")
        || darwin <= 8 && architecture == "x86_64")
}

struct Scan<'a> {
    progress: &'a Progress,
    prefixes: Vec<String>,
    id_check: bool,
    darwin: u32,
    host: &'static str,
    cache: BTreeMap<String, std::rc::Rc<Library>>,
    warned: BTreeSet<String>,
}

impl Scan<'_> {
    fn in_prefix(&self, path: &str) -> bool {
        self.prefixes
            .iter()
            .any(|prefix| !prefix.is_empty() && path.starts_with(prefix.as_str()))
    }

    fn broken(&mut self, binary: &str) -> bool {
        let slices = match fs::File::open(binary).map(|mut file| macho::slices(&mut file)) {
            Ok(Ok(Some(slices))) => slices,
            Ok(Ok(None)) => return false,
            Ok(Err(err)) => {
                self.progress
                    .warning(&format!("Error parsing file {binary}: {err:#}"));
                return false;
            }
            Err(err) => {
                self.progress
                    .warning(&format!("Error parsing file {binary}: {err}"));
                return false;
            }
        };
        let mut broken = false;
        for slice in &slices {
            let architecture = slice.architecture().unwrap_or("unknown");
            if self.id_check
                && let Some(id) = &slice.id
            {
                self.check_id(binary, architecture, &id.name);
            }
            if !runnable(architecture, self.darwin, self.host) {
                continue;
            }
            for load in &slice.loads {
                if load.command == LC_LOAD_WEAK_DYLIB {
                    continue;
                }
                let Some(path) = special_path(binary, &load.name) else {
                    continue;
                };
                match &*library(&path, &mut self.cache) {
                    Library::Cached | Library::NotMachO => {}
                    Library::Missing(err) => {
                        if self.warned.insert(path.clone()) {
                            self.progress.log(
                                "info",
                                &format!("Could not open {path}: {err} (referenced from {binary})"),
                            );
                        }
                        broken = true;
                    }
                    Library::Slices(library) => {
                        match library.iter().find(|candidate| candidate.cpu == slice.cpu) {
                            Some(candidate) => {
                                let provided = candidate.id.as_ref();
                                let (current, compatibility) = provided
                                    .map(|id| (id.current, id.compatibility))
                                    .unwrap_or((0, 0));
                                if load.current != current && load.compatibility > compatibility {
                                    self.progress.log(
                                        "info",
                                        &format!(
                                            "Incompatible library version: {binary} requires version {} or later, but {path} provides version {}",
                                            macho::format_version(load.compatibility),
                                            macho::format_version(compatibility)
                                        ),
                                    );
                                    broken = true;
                                }
                            }
                            None if self.in_prefix(&path) => broken = true,
                            None => {}
                        }
                    }
                }
            }
        }
        broken
    }

    fn check_id(&self, binary: &str, architecture: &str, id: &str) {
        let Some(path) = special_path(binary, id) else {
            self.progress.warning(&format!(
                "ID load command in {binary}, arch {architecture} contains a variable that could not be evaluated"
            ));
            return;
        };
        if !path.starts_with('/') {
            self.progress.warning(&format!(
                "ID load command in {binary}, arch {architecture} contains relative path"
            ));
        } else if !Path::new(&path).exists() {
            self.progress.warning(&format!(
                "ID load command in {binary}, arch {architecture} refers to non-existent file {path}"
            ));
        } else if fs::read(&path).ok() != fs::read(binary).ok() {
            self.progress.warning(&format!(
                "ID load command in {binary}, arch {architecture} refers to file {path}, which is a different file"
            ));
        }
    }
}

fn label(entry: &Entry) -> String {
    format!("{} @{}{}", entry.name, entry.full_version(), entry.variants)
}

/// Dependencies first, among the broken ports only; a cycle is broken by
/// taking the first remaining port, as upstream does.
fn rebuild_order(broken: Vec<Entry>, edges: &[(String, i64)]) -> Vec<Entry> {
    let mut remaining = broken;
    let mut ordered = Vec::new();
    while !remaining.is_empty() {
        let position = remaining
            .iter()
            .position(|entry| {
                !edges.iter().any(|(name, id)| {
                    *id == entry.id
                        && remaining.iter().any(|other| {
                            other.id != entry.id && other.name.eq_ignore_ascii_case(name)
                        })
                })
            })
            .unwrap_or(0);
        ordered.push(remaining.remove(position));
    }
    ordered
}

pub(super) fn run(
    out: &mut impl Write,
    index: &Index,
    registry_path: &Path,
    args: RevUpgrade,
    quiet: bool,
    root: &Path,
) -> Result<()> {
    let host = super::local::Host::new(root, Some(registry_path), quiet)?;
    let progress = host.progress.clone();
    let variable = |name: &str| {
        host.context
            .variables
            .get(name)
            .cloned()
            .unwrap_or_default()
    };
    let mode = variable("revupgrade_mode");
    let report_only = args.dry_run || mode == "report";
    let platform = crate::source::detect_platform()?;
    let cxx_stdlib = variable("cxx_stdlib");
    let wrong_stdlib = if cxx_stdlib == "libstdc++" {
        "libc++"
    } else {
        "libstdc++"
    };
    let mut attempts: BTreeMap<String, usize> = BTreeMap::new();
    loop {
        let Some(registry) = Registry::open(Some(registry_path))? else {
            progress.phase("No broken files found.");
            return Ok(());
        };
        let binaries = registry.active_binaries()?;
        let mut scan = Scan {
            progress: &progress,
            prefixes: vec![variable("prefix"), variable("applications_dir")],
            id_check: args.id_loadcmd_check,
            darwin: platform.darwin_major,
            host: platform.architecture,
            cache: BTreeMap::new(),
            warned: BTreeSet::new(),
        };
        let mut broken_files = BTreeSet::new();
        if !binaries.is_empty() {
            progress.phase("Scanning binaries for linking errors");
            for binary in &binaries {
                if scan.broken(binary) {
                    broken_files.insert(binary.clone());
                }
            }
        }
        let entries = registry.entries()?;
        let mut files_by_port: BTreeMap<i64, Vec<String>> = BTreeMap::new();
        if broken_files.is_empty() {
            progress.phase("No broken files found.");
        } else {
            let count = broken_files.len();
            let plural = if count == 1 { "" } else { "s" };
            progress.phase(&format!(
                "Found {count} broken file{plural}, matching files to ports"
            ));
            for file in &broken_files {
                match registry.owner(file, true)? {
                    Some(owner) => {
                        if let Some(entry) = entries
                            .iter()
                            .find(|entry| entry.active && entry.name == owner)
                        {
                            files_by_port
                                .entry(entry.id)
                                .or_default()
                                .push(file.clone());
                        }
                    }
                    None => {
                        progress.error(&format!("Broken file {file} doesn't belong to any port."))
                    }
                }
            }
        }
        let mut broken_ids: BTreeSet<i64> = files_by_port.keys().copied().collect();
        for id in registry.cxx_stdlib_mismatches(wrong_stdlib)? {
            if let Some(entry) = entries.iter().find(|entry| entry.id == id) {
                progress.log(
                    "info",
                    &format!("{} is using {wrong_stdlib} (this installation is configured to use {cxx_stdlib})", entry.name),
                );
            }
            broken_ids.insert(id);
        }
        if broken_ids.is_empty() {
            progress.phase("No broken ports found.");
            return Ok(());
        }
        let edges = registry.dependent_edges()?;
        drop(registry);
        let mut broken: Vec<Entry> = entries
            .iter()
            .filter(|entry| broken_ids.contains(&entry.id))
            .cloned()
            .collect();
        if !report_only {
            broken.retain(|entry| {
                let known = crate::query::lookup(index, &entry.name).is_ok();
                if !known {
                    progress.warning(&format!(
                        "No port {} found in the index; can't rebuild",
                        entry.name
                    ));
                }
                known
            });
            if broken.is_empty() {
                progress.phase(
                    "Broken files found, but all associated ports are not in the index and so cannot be rebuilt.",
                );
                return Ok(());
            }
        }
        for entry in &broken {
            let count = attempts.entry(entry.name.clone()).or_default();
            *count += 1;
            if *count > 3 {
                bail!(
                    "Port {} still broken after rebuilding {} times",
                    entry.name,
                    *count - 1
                );
            }
        }
        let count = broken.len();
        let plural = if count == 1 { "" } else { "s" };
        if report_only {
            progress.phase(&format!("Found {count} broken port{plural}:"));
            for entry in &broken {
                writeln!(out, "     {}", label(entry))?;
                match files_by_port.get(&entry.id) {
                    Some(files) => {
                        for file in files {
                            writeln!(out, "         {file}")?;
                        }
                    }
                    None => writeln!(out, "         (cxx_stdlib mismatch)")?,
                }
            }
            return Ok(());
        }
        progress.phase(&format!(
            "Found {count} broken port{plural}, determining rebuild order"
        ));
        let ordered = rebuild_order(broken, &edges);
        progress.phase("Rebuilding in order");
        for entry in &ordered {
            progress.message(&format!("     {}", label(entry)));
        }
        for entry in &ordered {
            let mut spec = PortSpec::named(&entry.name);
            for variant in entry.variants.split('+').filter(|v| !v.is_empty()) {
                spec.parse_modifier(&format!("+{variant}"))?;
            }
            super::lifecycle::uninstall_entry(registry_path, entry, &progress)?;
            super::install_exec::run_specs(
                out,
                index,
                Some(registry_path),
                vec![spec],
                quiet,
                root,
                true,
            )
            .map_err(|err| err.context(format!("Error rebuilding {}", entry.name)))?;
            if !entry.requested {
                let reader = Registry::open(Some(registry_path))?;
                if let Some(new) = reader
                    .map(|reader| reader.entries())
                    .transpose()?
                    .unwrap_or_default()
                    .into_iter()
                    .find(|new| new.active && new.name == entry.name)
                {
                    let mut writer = crate::registry::write::Writer::open(registry_path)?;
                    let mut transaction = writer.transaction()?;
                    transaction.set_requested(new.id, false)?;
                    transaction.commit()?;
                }
            }
        }
    }
}
