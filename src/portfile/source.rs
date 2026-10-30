//! Loading Portfiles from a caller-selected local ports tree.
use super::{Evaluation, PortGroups, evaluate_with_groups};
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
    evaluate_with_groups(
        &source,
        &variables,
        requested,
        PortGroups::from_directories(vec![root.join("_resources/port1.0/group")]),
    )
    .with_context(|| format!("{}: {}", requested.name, path.display()))
}
