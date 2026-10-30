//! CLI dispatch. Each action owns its options and presentation in a separate module.
mod contents;
mod dependencies;
mod dependents;
mod download;
mod info;
mod inspect;
mod install;
mod installed;
mod local;
mod location;
mod outdated;
mod provides;
mod search;
mod size;
mod space;
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
    /// Preview a binary installation plan with -y (installation not yet implemented)
    Install(install::Install),
    /// Show the recorded location of an installed port image
    Location(location::Location),
    /// Report disk space used by active ports
    Space(space::Space),
    /// Compare active installations with the available catalog
    Outdated(outdated::Outdated),
    /// Show installed ports that depend on a port
    Dependents(dependents::Dependents),
    /// Show recursive installed-port dependents
    Rdependents(dependents::RecursiveDependents),
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
    /// Display dependencies from local Portfiles, or cached metadata with --index
    Deps(dependencies::DependencyOptions),
    /// Display recursive dependencies from local Portfiles or --index
    Rdeps(dependencies::RecursiveDependencyOptions),
    /// Display variants from local Portfiles, or cached metadata with --index
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
        if let Command::Install(args) = &command {
            ensure!(
                args.dry_run,
                r#"command "install" execution is not implemented yet; use -y install to preview an indexed binary installation plan; no Tcl or MacPorts fallback is used"#
            );
        }
        let stdout = io::stdout();
        let mut out = io::BufWriter::new(stdout.lock());
        if let Command::Outdated(args) = command {
            outdated::run(&mut out, registry_file, args, quiet, verbose, || {
                self.index(index_file, quiet)
            })?;
        } else if let Command::Dependents(args) = command {
            dependents::run(&mut out, registry_file, args, None, quiet, verbose)?;
        } else if let Command::Rdependents(args) = command {
            dependents::run(
                &mut out,
                registry_file,
                args.selection,
                Some(args.full),
                quiet,
                verbose,
            )?;
        } else if let Command::Provides(args) = command {
            provides::run(&mut out, registry_file, args, quiet)?;
        } else if let Command::Location(args) = command {
            location::run(&mut out, registry_file, args, quiet)?;
        } else if let Command::Space(args) = command {
            space::run(&mut out, registry_file, args)?;
        } else if let Command::Contents(args) = command {
            contents::run(&mut out, registry_file, args, quiet)?;
        } else if let Command::Installed(args) = command {
            installed::run(&mut out, registry_file, args, quiet, verbose, || {
                self.index(index_file, quiet)
            })?;
        } else if matches!(command, Command::Version | Command::Platform) {
            match command {
                Command::Version => system::version(&mut out, quiet)?,
                Command::Platform => system::platform(&mut out, quiet)?,
                _ => unreachable!(),
            }
        } else if let Command::Download(args) = command {
            download::run(&mut out, args, quiet)?;
        } else if let Command::Variants(args) = command {
            let root = local::root(index_file, args.index, "variants")?;
            variants::run(
                &mut out,
                self.index(index_file, quiet)?,
                args,
                quiet,
                root.as_deref(),
            )?;
        } else if let Command::Deps(args) = command {
            let root = local::root(index_file, args.index, "deps")?;
            dependencies::run(
                &mut out,
                self.index(index_file, quiet)?,
                &args,
                None,
                quiet,
                verbose,
                root.as_deref(),
            )?;
        } else if let Command::Rdeps(args) = command {
            let root = local::root(index_file, args.options.index, "rdeps")?;
            dependencies::run(
                &mut out,
                self.index(index_file, quiet)?,
                &args.options,
                Some(args.full),
                quiet,
                verbose,
                root.as_deref(),
            )?;
        } else {
            // Offline index queries never initialize an async runtime or HTTP client.
            let index = self.index(index_file, quiet)?;
            match command {
                Command::Install(args) => {
                    let root = if let Some(path) = index_file {
                        Some(
                            path.parent()
                                .filter(|p| !p.as_os_str().is_empty())
                                .unwrap_or(Path::new(".")),
                        )
                    } else if Path::new("PortIndex").try_exists()? {
                        Some(Path::new("."))
                    } else {
                        None
                    };
                    install::run(&mut out, index, registry_file, args, quiet, root)?
                }
                Command::List { ports } => {
                    let ports = if ports.is_empty() {
                        vec!["all".into()]
                    } else {
                        ports
                    };
                    inspect::run(&mut out, index, &ports, quiet, "list")?;
                }
                Command::Echo(args) => inspect::run(&mut out, index, &args.ports, quiet, "echo")?,
                Command::Search(args) => search::run(&mut out, index, args, quiet)?,
                Command::Info(args) => info::run(&mut out, index, args, quiet, verbose)?,
                Command::Outdated(_)
                | Command::Dependents(_)
                | Command::Rdependents(_)
                | Command::Provides(_)
                | Command::Contents(_)
                | Command::Installed(_)
                | Command::Version
                | Command::Platform
                | Command::Download(_)
                | Command::Location(_)
                | Command::Space(_)
                | Command::Deps(_)
                | Command::Rdeps(_)
                | Command::Variants(_)
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
