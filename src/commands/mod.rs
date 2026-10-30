//! CLI dispatch. Each action owns its options and presentation in a separate module.
mod contents;
mod dependencies;
mod download;
mod info;
mod inspect;
mod installed;
mod provides;
mod search;
mod system;
mod variants;

use anyhow::{Result, bail, ensure};
use clap::Subcommand;
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Subcommand)]
pub enum Command {
    /// Find the active port that owns a filesystem path
    Provides(provides::Provides),
    /// List files recorded for an installed port (read-only)
    Contents(contents::Contents),
    /// List ports recorded in the installed-port registry (read-only)
    Installed(installed::Installed),
    /// Display the version of this standalone Rust implementation
    Version,
    /// Display the native MacPorts platform and architecture
    Platform,
    /// List ports from a local MacPorts PortIndex
    List {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        ports: Vec<String>,
    },
    /// Search names and descriptions (case insensitive by default)
    Search(search::Search),
    /// Display indexed port metadata
    Info(info::Info),
    /// Print selected port specifications
    Echo(inspect::Selection),
    /// Display direct dependencies from PortIndex (--index required)
    Deps(dependencies::DependencyOptions),
    /// Display recursive dependencies from PortIndex (--index required)
    Rdeps(dependencies::RecursiveDependencyOptions),
    /// Display indexed variants and their descriptions (--index required)
    Variants(variants::Variants),
    /// Download files concurrently from a checksummed JSON manifest (Rust extension)
    Download(download::Download),
    #[command(external_subcommand)]
    Unsupported(Vec<String>),
}

pub fn run(command: Command, index_file: Option<&Path>, quiet: bool, verbose: bool) -> Result<()> {
    Session::default().run(command, index_file, None, quiet, verbose)
}

/// A command chain shares its parsed catalog until the selected source changes.
#[derive(Default)]
pub struct Session {
    catalog: Option<(Option<PathBuf>, crate::index::Index)>,
}
impl Session {
    fn index(&mut self, path: Option<&Path>, quiet: bool) -> Result<&crate::index::Index> {
        if !self
            .catalog
            .as_ref()
            .is_some_and(|(cached, _)| cached.as_deref() == path)
        {
            self.catalog = Some((
                path.map(Path::to_path_buf),
                crate::source::load(path, quiet)?,
            ));
        }
        Ok(&self.catalog.as_ref().unwrap().1)
    }
    pub fn run(
        &mut self,
        command: Command,
        index_file: Option<&Path>,
        registry_file: Option<&Path>,
        quiet: bool,
        verbose: bool,
    ) -> Result<()> {
        if let Command::Unsupported(args) = &command {
            bail!(
                "command {:?} is not implemented in the standalone Rust engine; no Tcl or MacPorts fallback is used",
                args[0]
            );
        }
        if let Command::Variants(options) = &command {
            ensure!(
                options.index,
                "variant inspection without --index requires Portfile evaluation, which is not implemented; use variants --index"
            );
        }
        let indexed = match &command {
            Command::Deps(options) => Some(options.index),
            Command::Rdeps(options) => Some(options.options.index),
            _ => None,
        };
        ensure!(
            indexed != Some(false),
            "dependency inspection without --index requires Portfile evaluation, which is not implemented; use deps --index or rdeps --index"
        );
        let stdout = io::stdout();
        let mut out = io::BufWriter::new(stdout.lock());
        if let Command::Provides(args) = command {
            provides::run(&mut out, registry_file, args, quiet)?;
        } else if let Command::Contents(args) = command {
            contents::run(&mut out, registry_file, args, quiet)?;
        } else if let Command::Installed(args) = command {
            installed::run(&mut out, registry_file, args, quiet, verbose)?;
        } else if matches!(command, Command::Version | Command::Platform) {
            match command {
                Command::Version => system::version(&mut out, quiet)?,
                Command::Platform => system::platform(&mut out, quiet)?,
                _ => unreachable!(),
            }
        } else if let Command::Download(args) = command {
            download::run(&mut out, args, quiet)?;
        } else {
            // Offline index queries never initialize an async runtime or HTTP client.
            let index = self.index(index_file, quiet)?;
            match command {
                Command::List { ports } => {
                    let ports = if ports.is_empty() {
                        vec!["all".into()]
                    } else {
                        ports
                    };
                    inspect::run(&mut out, index, &ports, quiet, "list")?;
                }
                Command::Echo(args) => inspect::run(&mut out, index, &args.ports, quiet, "echo")?,
                Command::Variants(args) => variants::run(&mut out, index, args, quiet)?,
                Command::Search(args) => search::run(&mut out, index, args, quiet)?,
                Command::Info(args) => info::run(&mut out, index, args, quiet, verbose)?,
                Command::Deps(args) => {
                    dependencies::run(&mut out, index, &args, None, quiet, verbose)?
                }
                Command::Rdeps(args) => dependencies::run(
                    &mut out,
                    index,
                    &args.options,
                    Some(args.full),
                    quiet,
                    verbose,
                )?,
                Command::Provides(_)
                | Command::Contents(_)
                | Command::Installed(_)
                | Command::Version
                | Command::Platform
                | Command::Download(_)
                | Command::Unsupported(_) => unreachable!(),
            }
        }
        out.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chain_reuses_catalog_and_reloads_when_source_changes() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first");
        let second = directory.path().join("second");
        for (path, name) in [(&first, "first"), (&second, "second")] {
            let body = format!("name {name} version 1");
            std::fs::write(path, format!("{name} {}\n{body}\n", body.len() + 1)).unwrap();
        }
        let mut session = Session::default();
        assert!(
            session
                .index(Some(&first), true)
                .unwrap()
                .ports()
                .contains_key("first")
        );
        std::fs::remove_file(&first).unwrap();
        assert!(
            session
                .index(Some(&first), true)
                .unwrap()
                .ports()
                .contains_key("first")
        );
        assert!(
            session
                .index(Some(&second), true)
                .unwrap()
                .ports()
                .contains_key("second")
        );
        assert!(session.index(Some(&first), true).is_err());
    }
}
