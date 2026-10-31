//! Loading Portfiles from a caller-selected local ports tree.
use super::{Evaluation, PortGroups, Session};
use crate::{index::Port, spec::PortSpec};
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::{Component, Path},
};

/// `root` is the directory containing the selected PortIndex. Configuration and
/// platform variables remain explicit; this loader does not fetch missing code.
pub fn evaluate_from_tree(
    root: &Path,
    indexed: &Port,
    variables: &BTreeMap<String, String>,
    requested: &PortSpec,
) -> Result<Evaluation> {
    Ok(open_from_tree(root, indexed, variables, requested)?.into_metadata())
}

/// Open a persistent evaluator using the same bounded, tree-contained loader
/// as metadata queries. No hooks or external commands execute during opening.
pub fn open_from_tree(
    root: &Path,
    indexed: &Port,
    variables: &BTreeMap<String, String>,
    requested: &PortSpec,
) -> Result<Session> {
    open_tree(root, indexed, variables, requested, None, None, None)
}

/// Load a persistent Portfile with explicit environment values available during
/// evaluation. Shell execution remains disabled, as with `open_from_tree`.
pub fn open_from_tree_with_environment(
    root: &Path,
    indexed: &Port,
    variables: &BTreeMap<String, String>,
    requested: &PortSpec,
    environment: &BTreeMap<String, String>,
) -> Result<Session> {
    open_tree(
        root,
        indexed,
        variables,
        requested,
        Some(environment),
        None,
        None,
    )
}

/// Open a local-tree session with lazy archive metadata available to PortGroups.
pub fn open_from_tree_with_archive_sites(
    root: &Path,
    indexed: &Port,
    variables: &BTreeMap<String, String>,
    requested: &PortSpec,
    environment: &BTreeMap<String, String>,
    archives: super::ArchiveSites,
) -> Result<Session> {
    open_tree(
        root,
        indexed,
        variables,
        requested,
        Some(environment),
        Some(archives),
        None,
    )
}

/// Load a local session with registry lookup bound before evaluating source.
pub fn open_from_tree_with_registry(
    root: &Path,
    indexed: &Port,
    variables: &BTreeMap<String, String>,
    requested: &PortSpec,
    environment: &BTreeMap<String, String>,
    archives: super::ArchiveSites,
    registry: Option<&Path>,
) -> Result<Session> {
    open_tree(
        root,
        indexed,
        variables,
        requested,
        Some(environment),
        Some(archives),
        Some(super::registry_lookup::Source::from_path(registry)),
    )
}

fn open_tree(
    root: &Path,
    indexed: &Port,
    variables: &BTreeMap<String, String>,
    requested: &PortSpec,
    environment: Option<&BTreeMap<String, String>>,
    archives: Option<super::ArchiveSites>,
    registry: Option<super::registry_lookup::Source>,
) -> Result<Session> {
    let relative = Path::new(indexed.get("portdir"));
    ensure!(
        !relative.as_os_str().is_empty()
            && relative
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
        "{}: index has no valid relative portdir; use --index for indexed metadata",
        indexed.name
    );
    let root = root
        .canonicalize()
        .context("cannot resolve local ports tree")?;
    let path = root.join(relative).join("Portfile");
    let path = path.canonicalize().with_context(|| {
        format!(
            "cannot find local Portfile {}; use --index for indexed metadata",
            path.display()
        )
    })?;
    ensure!(
        path.starts_with(&root),
        "Portfile resolves outside the selected ports tree"
    );
    const MAX_SOURCE: u64 = 4 * 1024 * 1024;
    let metadata = path.metadata()?;
    ensure!(metadata.is_file(), "Portfile is not a regular file");
    ensure!(metadata.len() <= MAX_SOURCE, "Portfile exceeds 4 MiB limit");
    let file = File::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    ensure!(file.metadata()?.is_file(), "Portfile is not a regular file");
    let mut source = String::new();
    file.take(MAX_SOURCE + 1)
        .read_to_string(&mut source)
        .with_context(|| format!("cannot read {}", path.display()))?;
    ensure!(
        source.len() as u64 <= MAX_SOURCE,
        "Portfile exceeds 4 MiB limit"
    );
    let mut variables = variables.clone();
    variables.insert(
        "portpath".into(),
        path.parent().unwrap().to_string_lossy().into_owned(),
    );
    super::open(
        &source,
        &variables,
        requested,
        PortGroups::from_tree(&root),
        environment,
        archives,
        registry,
    )
    .with_context(|| format!("{}: {}", requested.name, path.display()))
}
