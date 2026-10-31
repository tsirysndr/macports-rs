//! Port-directory commands (`action_portcmds`), `notes`, and requested flags.
use crate::{index::Index, query, registry, registry::write::Writer};
use anyhow::{Context, Result, bail};
use clap::Args;
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Args)]
pub struct Ports {
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    pub(super) ports: Vec<String>,
}

/// `$portdbpath/<kind>/<mangled port dir>/<port>`, as upstream names build and
/// log directories.
fn port_path(portdbpath: &str, kind: &str, portdir: &Path, name: &str) -> PathBuf {
    Path::new(portdbpath)
        .join(kind)
        .join(portdir.to_string_lossy().replace('/', "_"))
        .join(name)
}

pub(super) fn run(
    out: &mut impl Write,
    index: &Index,
    action: &str,
    args: Ports,
    root: &Path,
) -> Result<()> {
    let root = std::fs::canonicalize(root)?;
    let host = super::local::Host::new(&root, None, true)?;
    let portdbpath = host
        .context
        .variables
        .get("portdbpath")
        .cloned()
        .unwrap_or_default();
    for spec in query::specifications(index, &args.ports)? {
        let port = query::lookup(index, &spec.name)?;
        let portdir = root.join(port.get("portdir"));
        let portfile = portdir.join("Portfile");
        if !portfile.is_file() {
            bail!("Could not read {}", portfile.display());
        }
        match action {
            "cat" => out.write_all(&std::fs::read(&portfile)?)?,
            "dir" => writeln!(out, "{}", portdir.display())?,
            "url" => writeln!(out, "file://{}", portdir.display())?,
            "file" => writeln!(out, "{}", portfile.display())?,
            "work" => {
                let work = port_path(&portdbpath, "build", &portdir, &port.name).join("work");
                if work.exists() {
                    writeln!(out, "{}", work.display())?;
                }
            }
            "logfile" => {
                let log = port_path(&portdbpath, "logs", &portdir, &port.name).join("main.log");
                if log.is_file() {
                    writeln!(out, "{}", log.display())?;
                } else {
                    bail!("Log file for port {} not found", port.name);
                }
            }
            "edit" => {
                out.flush()?;
                let editor = ["MP_EDITOR", "VISUAL", "EDITOR"]
                    .iter()
                    .find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
                    .unwrap_or_else(|| "/usr/bin/vi".into());
                let status = Command::new("/bin/sh")
                    .arg("-c")
                    .arg(format!("{editor} \"$1\""))
                    .arg("sh")
                    .arg(&portfile)
                    .status()
                    .with_context(|| format!("unable to invoke editor {editor}"))?;
                if !status.success() {
                    bail!("unable to invoke editor {editor}: {status}");
                }
            }
            "gohome" => {
                let homepage = port.get("homepage");
                if homepage.is_empty() {
                    bail!("No homepage for {}", port.name);
                }
                let status = Command::new("/usr/bin/open").arg(homepage).status()?;
                if !status.success() {
                    bail!("unable to invoke browser using /usr/bin/open: {status}");
                }
            }
            other => bail!("unsupported port command {other}"),
        }
    }
    Ok(())
}

pub(super) fn notes(
    out: &mut impl Write,
    index: &Index,
    args: Ports,
    root: Option<&Path>,
) -> Result<()> {
    let host = root
        .map(|root| super::local::Host::new(root, None, true))
        .transpose()?;
    for spec in query::specifications(index, &args.ports)? {
        let indexed = query::lookup(index, &spec.name)?;
        let evaluated = match (root, &host) {
            (Some(root), Some(host)) => Some(host.evaluate(root, indexed, &spec)?),
            _ => None,
        };
        let port = evaluated
            .as_ref()
            .map(|value| &value.port)
            .unwrap_or(indexed);
        let notes = crate::tcl_list::parse(port.get("notes"))?;
        if notes.is_empty() {
            writeln!(out, "---> {} has no notes.", port.name)?;
        } else {
            writeln!(out, "---> {} has the following notes:", port.name)?;
            for note in notes {
                for line in note.lines() {
                    writeln!(out, "  {}", line.trim_start())?;
                }
            }
        }
    }
    Ok(())
}

pub(super) fn set_requested(registry_path: &Path, args: Ports, requested: bool) -> Result<()> {
    let entries = registry::Registry::open(Some(registry_path))?
        .context("no ports are installed")?
        .entries()?;
    let mut writer = Writer::open(registry_path)?;
    for spec in registry::specifications(&args.ports)? {
        let version = spec.composite_version();
        let matches: Vec<_> = entries
            .iter()
            .filter(|entry| {
                entry.name.eq_ignore_ascii_case(&spec.name)
                    && (version.is_empty() || version == entry.full_version())
            })
            .collect();
        if matches.is_empty() {
            bail!(
                "{} is not installed",
                format!("{} {version}", spec.name).trim()
            );
        }
        let mut transaction = writer.transaction()?;
        for entry in matches {
            transaction.set_requested(entry.id, requested)?;
        }
        transaction.commit()?;
    }
    Ok(())
}

#[derive(Args)]
pub struct Log {
    /// Only show messages from this phase
    #[arg(long)]
    phase: Option<String>,
    /// Show messages up to this priority (error, warn, msg, notice, info, debug)
    #[arg(long)]
    level: Option<String>,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}

const PRIORITIES: [&str; 7] = ["error", "warn", "msg", "notice", "info", "debug", "any"];

/// `port log`: print a port's main.log filtered by phase and priority.
pub(super) fn log(out: &mut impl Write, index: &Index, args: Log, root: &Path) -> Result<()> {
    let root = std::fs::canonicalize(root)?;
    let host = super::local::Host::new(&root, None, true)?;
    let portdbpath = host
        .context
        .variables
        .get("portdbpath")
        .cloned()
        .unwrap_or_default();
    let priorities: Option<&[&str]> = args.level.as_deref().map(|level| {
        match PRIORITIES.iter().position(|priority| *priority == level) {
            Some(index) => &PRIORITIES[..=index],
            None => &[][..],
        }
    });
    for spec in query::specifications(index, &args.ports)? {
        let port = query::lookup(index, &spec.name)?;
        let portdir = root.join(port.get("portdir"));
        let path = port_path(&portdbpath, "logs", &portdir, &port.name).join("main.log");
        let Ok(text) = std::fs::read_to_string(&path) else {
            bail!("Log file for port {} not found", port.name);
        };
        for line in text.lines() {
            let Some((priority, rest)) =
                line.strip_prefix(':').and_then(|line| line.split_once(':'))
            else {
                continue;
            };
            let Some((phase, message)) = rest.split_once(' ') else {
                continue;
            };
            let lowercase =
                |word: &str| !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase());
            let priority_matches = match priorities {
                Some(allowed) => priority == "any" || allowed.contains(&priority),
                None => lowercase(priority),
            };
            let phase_matches = match &args.phase {
                Some(wanted) => phase == "any" || phase == wanted,
                None => lowercase(phase),
            };
            if priority_matches && phase_matches {
                let prefix = match priority {
                    "debug" => "DEBUG: ",
                    "error" => "Error: ",
                    "warn" => "Warning: ",
                    _ => "",
                };
                writeln!(out, "{prefix}{message}")?;
            }
        }
    }
    Ok(())
}
