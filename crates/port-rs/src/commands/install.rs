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
    /// Install the requested ports' images without activating them
    #[arg(long = "no-activate")]
    pub(super) no_activate: bool,
    /// Do not mark the ports as requested
    #[arg(long)]
    pub(super) unrequested: bool,
    /// Try to install ports that are known to fail
    #[arg(long = "allow-failing")]
    pub(super) allow_failing: bool,
    /// Install replaced ports instead of their replacements
    #[arg(long = "no-replace")]
    pub(super) no_replace: bool,
    /// Skip checking for broken binaries after installing
    #[arg(long = "no-rev-upgrade")]
    pub(super) no_rev_upgrade: bool,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    pub(super) ports: Vec<String>,
}

pub(super) fn run(
    out: &mut impl Write,
    index: &Index,
    registry_file: Option<&Path>,
    args: Install,
    quiet: bool,
    root: Option<&Path>,
) -> Result<()> {
    let mut specs = query::specifications(index, &args.ports)?;
    let local = if let Some(root) = root {
        let mut available = root.join("_resources/port1.0/group").try_exists()?;
        for spec in &specs {
            let port = query::lookup(index, &spec.name)?;
            let relative = Path::new(port.get("portdir"));
            if !relative.as_os_str().is_empty()
                && relative
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_)))
            {
                available |= root.join(relative).join("Portfile").try_exists()?;
            }
        }
        available.then_some(root)
    } else {
        None
    };
    let evaluated = local
        .map(|root| {
            let host = super::local::Host::new(root, registry_file, quiet)?;
            for spec in &mut specs {
                host.context.variants.apply(spec);
            }
            let registry = host.context.open_registry(registry_file)?;
            install::plan_sessions(index, &specs, registry.as_ref(), |port, spec| {
                host.open(root, port, spec)
            })
        })
        .transpose()?;
    let items = if let Some(evaluated) = &evaluated {
        evaluated.plan.items().collect()
    } else {
        install::plan(index, &specs, &registry::read(registry_file)?)?
    };
    if !quiet {
        writeln!(
            out,
            "Preliminary binary installation plan ({}):",
            if evaluated.is_some() {
                "evaluated local Portfiles"
            } else {
                "indexed metadata"
            }
        )?;
        if evaluated.is_some() {
            writeln!(
                out,
                "PortSystem initialization and build/activation hooks are not executed."
            )?;
        }
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
            Action::Resolve => "resolve",
        };
        writeln!(
            out,
            "{action:<8} {} @{}{}{}{}{}",
            item.port.name,
            item.port.version(),
            item.variants,
            if item.requested {
                " (requested)"
            } else {
                " (dependency)"
            },
            if item.variants_resolved {
                ""
            } else {
                " [default variants unresolved]"
            },
            if item.provider_fallbacks.is_empty() {
                String::new()
            } else {
                format!(
                    " [provider checks pending: {}]",
                    item.provider_fallbacks.join(", ")
                )
            }
        )?;
    }
    Ok(())
}
