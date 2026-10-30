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
    root: Option<&Path>,
) -> Result<()> {
    let mut specs = query::specifications(index, &args.ports)?;
    let registry = registry::Registry::open(registry_file)?;
    let entries = registry
        .as_ref()
        .map(registry::Registry::entries)
        .transpose()?
        .unwrap_or_default();
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
            let variables = super::local::variables(Some(root))?;
            let environment = std::env::vars_os()
                .map(|(name, value)| {
                    Ok((
                        name.into_string().map_err(|_| {
                            anyhow::anyhow!("startup environment name is not UTF-8")
                        })?,
                        value.into_string().map_err(|_| {
                            anyhow::anyhow!("startup environment value is not UTF-8")
                        })?,
                    ))
                })
                .collect::<Result<_>>()?;
            // SAFETY: getuid has no arguments and does not mutate process state.
            let uid = unsafe { libc::getuid() };
            let paths = crate::config::Paths::discover(
                "/opt/local/etc/macports".into(),
                std::env::current_dir()?,
                &environment,
                uid,
            )?;
            let context = crate::config::Context::new(
                &paths.load()?,
                variables,
                &environment,
                uid,
                &crate::install::proxy::Settings::default(),
            )?;
            let progress = crate::progress::Progress::new(quiet);
            for warning in &context.warnings {
                progress.warning(warning);
            }
            for spec in &mut specs {
                context.variants.apply(spec);
            }
            install::plan_sessions(index, &specs, registry.as_ref(), |port, spec| {
                crate::portfile::open_from_tree_with_environment(
                    root,
                    port,
                    &context.variables,
                    spec,
                    &context.environment,
                )
            })
        })
        .transpose()?;
    let items = if let Some(evaluated) = &evaluated {
        evaluated.plan.items().collect()
    } else {
        install::plan(index, &specs, &entries)?
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
