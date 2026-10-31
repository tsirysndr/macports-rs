//! Pseudo-portnames over the registry and ports tree (`installed`,
//! `outdated`, `leaves`, `current`, Portfile directories, ...).
use crate::{
    index::Index,
    registry::{self, Entry, Registry, expression::Missing},
    spec::PortSpec,
};
use anyhow::{Context, Result, bail};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Mutex,
};

fn split_variants(spec: &mut PortSpec, variants: &str) -> Result<()> {
    let mut rest = variants;
    while let Some(start) = rest.find(['+', '-']) {
        let tail = &rest[start + 1..];
        let end = tail.find(['+', '-']).unwrap_or(tail.len());
        spec.parse_modifier(&rest[start..start + 1 + end])?;
        rest = &tail[end..];
    }
    Ok(())
}

/// Installed images as specifications; `exact` keeps `@version_revision`.
fn specs(entries: &[&Entry], exact: bool) -> Result<Vec<PortSpec>> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        if !exact && !seen.insert(entry.name.to_ascii_lowercase()) {
            continue;
        }
        let mut spec = PortSpec::named(&entry.name);
        if exact {
            spec.parse_version(&format!("@{}_{}", entry.version, entry.revision))?;
        }
        split_variants(&mut spec, &entry.variants)?;
        result.push(spec);
    }
    result.sort_by(|a, b| crate::order::dictionary(&a.name, &b.name));
    Ok(result)
}

/// The port whose Portfile is in `directory`, by its place in the tree.
fn port_in_directory(index: &Index, root: &Path, directory: &Path) -> Result<String> {
    let directory = std::fs::canonicalize(directory)
        .with_context(|| format!("cannot access {}", directory.display()))?;
    let directory = if directory.is_file() {
        directory
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or(directory)
    } else {
        directory
    };
    if !directory.join("Portfile").is_file() {
        bail!("To use the current port, you must be in a port's directory.");
    }
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    if let Ok(relative) = directory.strip_prefix(&root) {
        let relative = relative.to_string_lossy();
        if let Some((name, _)) = index
            .ports()
            .iter()
            .find(|(_, port)| port.get("portdir") == relative)
        {
            return Ok(name.clone());
        }
    }
    let text = std::fs::read_to_string(directory.join("Portfile"))?;
    let name = text
        .lines()
        .find_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some("name")).then(|| words.next().map(str::to_string))?
        })
        .context("cannot determine the port name of this Portfile")?;
    if index.lookup(&name).is_some() {
        return Ok(name);
    }
    bail!(
        "{} is outside the selected ports tree; use --index-file with the tree that contains it",
        directory.display()
    )
}

pub(super) fn resolver(
    registry_path: PathBuf,
    index_path: Option<PathBuf>,
) -> Box<crate::query::PseudoResolver> {
    let tree = index_path
        .as_ref()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."));
    let index: Mutex<Option<std::sync::Arc<Index>>> = Mutex::new(None);
    let load = move || -> Result<std::sync::Arc<Index>> {
        let mut cached = index.lock().expect("index lock");
        if let Some(index) = cached.as_ref() {
            return Ok(index.clone());
        }
        let loaded = std::sync::Arc::new(crate::source::load(index_path.as_deref(), true)?);
        *cached = Some(loaded.clone());
        Ok(loaded)
    };
    Box::new(move |name: &str, exact: bool| {
        let read = || -> Result<(Option<Registry>, Vec<Entry>)> {
            let registry = if registry_path.exists() {
                Registry::open(Some(&registry_path))?
            } else {
                None
            };
            let entries = registry
                .as_ref()
                .map(|registry| registry.entries())
                .transpose()?
                .unwrap_or_default();
            Ok((registry, entries))
        };
        match name {
            "installed" | "active" | "inactive" | "actinact" | "requested" | "unrequested"
            | "leaves" | "rleaves" | "obsolete" => {
                let (registry, entries) = read()?;
                let index = if name == "obsolete" {
                    Some(load()?)
                } else {
                    None
                };
                let selected = registry::expression::select(
                    &entries,
                    &[name.to_string()],
                    Missing::Empty,
                    registry.as_ref(),
                    || {
                        let index = index.clone().context("catalog is required")?;
                        Ok(entries
                            .iter()
                            .filter(|entry| index.lookup(&entry.name).is_none())
                            .map(|entry| entry.name.to_ascii_lowercase())
                            .collect())
                    },
                )?;
                Ok(Some(specs(&selected, exact)?))
            }
            "outdated" => {
                let index = load()?;
                let outdated = super::outdated::outdated_entries(Some(&registry_path), &[], &index)
                    .unwrap_or_default();
                let refs: Vec<&Entry> = outdated.iter().collect();
                Ok(Some(specs(&refs, exact)?))
            }
            "uninstalled" => {
                let index = load()?;
                let (_, entries) = read()?;
                let installed: HashSet<String> = entries
                    .iter()
                    .map(|e| e.name.to_ascii_lowercase())
                    .collect();
                Ok(Some(
                    index
                        .ports()
                        .keys()
                        .filter(|name| !installed.contains(&name.to_ascii_lowercase()))
                        .map(PortSpec::named)
                        .collect(),
                ))
            }
            other => {
                let index = load()?;
                let directory = if other == "current" {
                    std::env::current_dir()?
                } else if let Some(path) = other.strip_prefix("file://") {
                    PathBuf::from(path)
                } else {
                    PathBuf::from(other)
                };
                let name = port_in_directory(&index, &tree, &directory)?;
                Ok(Some(vec![PortSpec::named(name)]))
            }
        }
    })
}
