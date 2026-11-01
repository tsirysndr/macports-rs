use crate::{index::Index, query};
use anyhow::Result;
use clap::Args;
use std::{io::Write, path::Path};

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
    root: Option<&Path>,
) -> Result<()> {
    let specs = query::specifications(index, &args.ports)?;
    let host = root
        .map(|root| super::local::Host::new(root, None, true))
        .transpose()?;
    let evaluated = root
        .zip(host.as_ref())
        .map(|(root, host)| {
            specs
                .iter()
                .map(|spec| host.evaluate(root, query::lookup(index, &spec.name)?, spec))
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?;
    let ports = specs
        .iter()
        .enumerate()
        .map(|(position, spec)| {
            if let Some(evaluated) = &evaluated {
                Ok(&evaluated[position].port)
            } else {
                query::lookup(index, &spec.name)
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let filter = crate::dependencies::Filter {
        no_build: args.no_build,
        no_test: args.no_test,
    };
    let output = crate::dependencies::OutputOptions {
        quiet,
        verbose,
        width: crate::output::terminal_width(),
    };
    if let Some(full) = full {
        let graphs = ports
            .iter()
            .zip(&specs)
            .map(|(port, spec)| {
                if let Some(root) = root {
                    crate::dependencies::Graph::resolve(port, |name| {
                        if name.eq_ignore_ascii_case(&port.name) {
                            return crate::dependencies::of(port, filter);
                        }
                        let indexed = query::lookup(index, name)?;
                        // Native action_deps passes the root's explicit variations
                        // and options to dependencies, but not its version constraint.
                        let mut requested = spec.clone();
                        requested.name = indexed.name.clone();
                        requested.version = None;
                        let value = host
                            .as_ref()
                            .expect("local evaluation has a host")
                            .evaluate(root, indexed, &requested)?;
                        crate::dependencies::of(&value.port, filter)
                    })
                } else {
                    crate::dependencies::Graph::build(index, &spec.name, filter)
                }
            })
            .collect::<Result<Vec<_>>>()?;
        for (i, graph) in graphs.iter().enumerate() {
            if i > 0 {
                writeln!(out, "--")?;
            }
            crate::dependencies::recursive(out, graph, full, output)?;
        }
    } else {
        let prepared = ports
            .into_iter()
            .map(|port| crate::dependencies::Direct::prepare(port, filter, output))
            .collect::<Result<Vec<_>>>()?;
        for (i, dependencies) in prepared.iter().enumerate() {
            if i > 0 {
                writeln!(out, "--")?;
            }
            dependencies.write(out)?;
        }
    }
    Ok(())
}
