//! `port select` (mportselect): switch `${prefix}/etc/select/<group>` symlink sets.
use anyhow::{Context, Result, bail, ensure};
use clap::Args;
use std::{
    fs,
    io::Write,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};

#[derive(Args)]
pub struct Select {
    /// List the available versions of a group
    #[arg(long)]
    list: bool,
    /// Select a version of a group
    #[arg(long)]
    set: bool,
    /// Show the selected version of a group
    #[arg(long)]
    show: bool,
    /// Show every group with its selection and options
    #[arg(long)]
    summary: bool,
    arguments: Vec<String>,
}

fn group(prefix: &Path, name: &str) -> Result<PathBuf> {
    let path = prefix.join("etc/select").join(name);
    ensure!(
        path.is_dir(),
        "The specified group '{name}' does not exist."
    );
    Ok(path)
}

fn versions(directory: &Path) -> Result<Vec<String>> {
    let mut versions: Vec<String> = fs::read_dir(directory)
        .map_err(|_| {
            anyhow::anyhow!(
                "No configurations associated with '{}' were found.",
                directory.display()
            )
        })?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "base" && name != "current")
        .collect();
    versions.sort();
    Ok(versions)
}

fn selected(directory: &Path) -> String {
    fs::read_link(directory.join("current"))
        .map(|target| target.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "none".into())
}

fn lines(path: &Path) -> Result<Vec<String>> {
    let text = fs::read_to_string(path)?;
    Ok(text
        .strip_suffix('\n')
        .unwrap_or(&text)
        .split('\n')
        .map(str::to_string)
        .collect())
}

fn replace_link(target: &Path, source: &Path) -> Result<()> {
    if fs::symlink_metadata(target).is_ok() {
        fs::remove_file(target)?;
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    symlink(source, target).with_context(|| format!("cannot link {}", target.display()))
}

fn set(prefix: &Path, directory: &Path, version: &str) -> Result<()> {
    let invalid = || anyhow::anyhow!("The specified version '{version}' is not valid.");
    if version.is_empty() || version == "base" || version == "current" || version.contains('/') {
        return Err(invalid());
    }
    let sources = lines(&directory.join(version)).map_err(|_| invalid())?;
    let targets = lines(&directory.join("base")).map_err(|_| {
        anyhow::anyhow!(
            "The configuration file '{}' could not be opened.",
            directory.join("base").display()
        )
    })?;
    for (index, target) in targets.iter().enumerate() {
        let target = prefix.join(target.trim_start_matches('/'));
        match sources.get(index).map(String::as_str).unwrap_or("") {
            "-" => {
                if fs::symlink_metadata(&target).is_ok() {
                    fs::remove_file(&target)?;
                }
            }
            source if source.starts_with('/') => replace_link(&target, Path::new(source))?,
            source => replace_link(&target, &prefix.join(source))?,
        }
    }
    replace_link(&directory.join("current"), Path::new(version))
}

pub(super) fn run(out: &mut impl Write, args: Select, quiet: bool) -> Result<()> {
    let prefix = super::local::configured_prefix();
    let commands = [args.list, args.set, args.show, args.summary]
        .iter()
        .filter(|flag| **flag)
        .count();
    ensure!(
        commands <= 1,
        "Multiple commands were specified. Only one command may be specified at a time."
    );
    let arguments = &args.arguments;
    if arguments.is_empty() && !args.summary {
        bail!(
            "Incorrect usage. Correct synopsis is one of:\n  port select [--list|--show] <group>\n  port select [--set] <group> <version>\n  port select --summary"
        );
    }
    if args.summary {
        let root = prefix.join("etc/select");
        let mut groups: Vec<String> = fs::read_dir(&root)
            .map_err(|_| anyhow::anyhow!("No ports with the select option were found."))?
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        groups.sort();
        let mut rows = Vec::new();
        let (mut name_width, mut selected_width) = (4, 8);
        for name in groups {
            let directory = root.join(&name);
            let Ok(mut options) = versions(&directory) else {
                continue;
            };
            options.retain(|option| option != "none");
            options.push("none".into());
            let current = selected(&directory);
            name_width = name_width.max(name.len());
            selected_width = selected_width.max(current.len());
            rows.push((name, current, options.join(" ")));
        }
        if !quiet {
            writeln!(
                out,
                "{:<name_width$}  {:<selected_width$}  Options",
                "Name", "Selected"
            )?;
            writeln!(
                out,
                "{:<name_width$}  {:<selected_width$}  =======",
                "====", "========"
            )?;
        }
        for (name, current, options) in rows {
            writeln!(
                out,
                "{name:<name_width$}  {current:<selected_width$}  {options}"
            )?;
        }
        return Ok(());
    }
    let name = &arguments[0];
    let directory = group(&prefix, name)?;
    if args.set || (!args.list && !args.show && arguments.len() > 1) {
        let version = arguments
            .get(1)
            .context("The 'set' command expects two arguments: <group>, <version>")?;
        write!(out, "Selecting '{version}' for '{name}' ")?;
        match set(&prefix, &directory, version) {
            Ok(()) => writeln!(out, "succeeded. '{version}' is now active.")?,
            Err(err) => {
                writeln!(out, "failed: {err}")?;
                out.flush()?;
                bail!("select failed");
            }
        }
    } else if args.show {
        writeln!(
            out,
            "The currently selected version for '{name}' is '{}'.",
            selected(&directory)
        )?;
    } else {
        let current = selected(&directory);
        if !quiet {
            writeln!(out, "Available versions for {name}:")?;
        }
        for version in versions(&directory)? {
            let marker = if version == current { " (active)" } else { "" };
            writeln!(out, "\t{version}{marker}")?;
        }
    }
    Ok(())
}
