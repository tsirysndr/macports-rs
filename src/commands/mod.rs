//! CLI dispatch. Each action owns its options and presentation in a separate module.
mod dependencies;
mod download;
mod info;
mod inspect;
mod search;

use anyhow::{Result, bail, ensure};
use clap::Subcommand;
use std::{
    io::{self, Write},
    path::Path,
};

#[derive(Subcommand)]
pub enum Command {
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
    /// Display indexed variants
    Variants(inspect::Selection),
    /// Download files concurrently from a checksummed JSON manifest (Rust extension)
    Download(download::Download),
    #[command(external_subcommand)]
    Unsupported(Vec<String>),
}

pub fn run(command: Command, index_file: Option<&Path>, quiet: bool, verbose: bool) -> Result<()> {
    if let Command::Unsupported(args) = &command {
        bail!(
            "command {:?} is not implemented in the standalone Rust engine; no Tcl or MacPorts fallback is used",
            args[0]
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
    if let Command::Download(args) = command {
        download::run(&mut out, args, quiet)?;
    } else {
        // Offline index queries never initialize an async runtime or HTTP client.
        let index = crate::source::load(index_file, quiet)?;
        match command {
            Command::List { ports } => {
                let ports = if ports.is_empty() {
                    vec!["all".into()]
                } else {
                    ports
                };
                inspect::run(&mut out, &index, &ports, quiet, "list")?;
            }
            Command::Echo(args) => inspect::run(&mut out, &index, &args.ports, quiet, "echo")?,
            Command::Variants(args) => {
                inspect::run(&mut out, &index, &args.ports, quiet, "variants")?
            }
            Command::Search(args) => search::run(&mut out, &index, args, quiet)?,
            Command::Info(args) => info::run(&mut out, &index, args, quiet, verbose)?,
            Command::Deps(args) => {
                dependencies::run(&mut out, &index, &args, None, quiet, verbose)?
            }
            Command::Rdeps(args) => dependencies::run(
                &mut out,
                &index,
                &args.options,
                Some(args.full),
                quiet,
                verbose,
            )?,
            Command::Download(_) | Command::Unsupported(_) => unreachable!(),
        }
    }
    out.flush()?;
    Ok(())
}
