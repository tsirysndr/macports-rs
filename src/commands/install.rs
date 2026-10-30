use crate::{
    index::Index,
    install::{self, Action},
    query, registry,
};
use anyhow::Result;
use clap::Args;
use std::{io::Write, path::Path};

#[derive(Args)]
pub struct Install {
    #[arg(skip)]
    pub dry_run: bool,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}

pub(super) fn run(
    out: &mut impl Write,
    index: &Index,
    registry_file: Option<&Path>,
    args: Install,
    quiet: bool,
) -> Result<()> {
    let specs = query::specifications(index, &args.ports)?;
    let entries = registry::read(registry_file)?;
    let items = install::plan(index, &specs, &entries)?;
    if !quiet {
        writeln!(
            out,
            "Preliminary binary installation plan (indexed default variants):"
        )?;
        writeln!(
            out,
            "Archive availability and signatures are not checked; no packages will be installed."
        )?;
    }
    for item in items {
        let action = match item.action {
            Action::Install => "install",
            Action::Activate => "activate",
            Action::AlreadyActive => "active",
        };
        writeln!(
            out,
            "{action:<8} {} @{}{}{}",
            item.port.name,
            item.port.version(),
            item.variants,
            if item.requested {
                " (requested)"
            } else {
                " (dependency)"
            }
        )?;
    }
    Ok(())
}
