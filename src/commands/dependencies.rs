use crate::{index::Index, query};
use anyhow::Result;
use clap::Args;
use std::io::Write;

#[derive(Args)]
pub struct DependencyOptions {
    /// Read dependency metadata from PortIndex without evaluating Portfiles
    #[arg(long)]
    pub(super) index: bool,
    /// Exclude fetch, extract, patch, and build dependencies
    #[arg(long)]
    no_build: bool,
    /// Exclude test dependencies
    #[arg(long)]
    no_test: bool,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}
#[derive(Args)]
pub struct RecursiveDependencyOptions {
    #[command(flatten)]
    pub(super) options: DependencyOptions,
    /// Repeat shared subtrees and annotate dependency cycles
    #[arg(long)]
    pub(super) full: bool,
}
pub(super) fn run(
    out: &mut impl Write,
    index: &Index,
    args: &DependencyOptions,
    full: Option<bool>,
    quiet: bool,
    verbose: bool,
) -> Result<()> {
    let specs = query::specifications(index, &args.ports)?;
    let filter = crate::dependencies::Filter {
        no_build: args.no_build,
        no_test: args.no_test,
    };
    let output = crate::dependencies::OutputOptions {
        quiet,
        verbose,
        width: usize::from(console::Term::stdout().size().1).max(40),
    };
    if let Some(full) = full {
        let graphs = specs
            .iter()
            .map(|spec| crate::dependencies::Graph::build(index, &spec.name, filter))
            .collect::<Result<Vec<_>>>()?;
        for (i, graph) in graphs.iter().enumerate() {
            if i > 0 {
                writeln!(out, "--")?;
            }
            crate::dependencies::recursive(out, graph, full, output)?;
        }
    } else {
        let ports = specs
            .iter()
            .map(|spec| query::lookup(index, &spec.name))
            .collect::<Result<Vec<_>>>()?;
        for (i, port) in ports.into_iter().enumerate() {
            if i > 0 {
                writeln!(out, "--")?;
            }
            crate::dependencies::direct(out, port, filter, output)?;
        }
    }
    Ok(())
}
