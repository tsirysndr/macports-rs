use crate::{index::Index, query};
use anyhow::{Result, ensure};
use clap::Args;
use std::{borrow::Cow, io::Write, path::Path};

#[derive(Args)]
pub struct Info {
    /// Read metadata from PortIndex without evaluating Portfiles
    #[arg(long)]
    index: bool,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
    #[command(flatten)]
    fields: crate::fields::InfoFields,
    /// Tab-separated output in the requested field order
    #[arg(long)]
    line: bool,
    /// Format fields as a human-readable summary
    #[arg(long)]
    pretty: bool,
}

pub(super) fn run(
    out: &mut impl Write,
    index: &Index,
    s: Info,
    quiet: bool,
    verbose: bool,
    root: Option<&Path>,
) -> Result<()> {
    let specs = query::specifications(index, &s.ports)?;
    ensure!(
        s.index || root.is_some() || specs.iter().all(|spec| !spec.has_modifiers()),
        "evaluating version/variant/option-specific metadata requires a local ports tree; use info --index for available indexed metadata"
    );
    // Upstream opens each Portfile; only variant defaults and modified specs
    // differ from the index, so evaluate just when the output needs them.
    let pretty = s.pretty || (s.fields.ordered.is_empty() && !s.line);
    let variants = s.fields.ordered.is_empty()
        || s.fields
            .ordered
            .iter()
            .any(|field| crate::fields::canonical(field) == "variants");
    let host = root
        .filter(|_| !s.index)
        .map(|root| super::local::Host::new(root, None, true))
        .transpose()?;
    let ports = specs
        .iter()
        .map(|spec| {
            let indexed = query::lookup(index, &spec.name)?;
            match (root, &host) {
                (Some(root), Some(host)) if spec.has_modifiers() || (pretty && variants) => {
                    Ok(Cow::Owned(host.evaluate(root, indexed, spec)?.port))
                }
                _ => Ok(Cow::Borrowed(indexed)),
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let options = crate::info::Options {
        fields: &s.fields.ordered,
        line: s.line,
        pretty: s.pretty,
        quiet,
        verbose,
        width: crate::output::terminal_width(),
    };
    for (i, (spec, port)) in specs.iter().zip(ports).enumerate() {
        if i > 0 && !s.line {
            writeln!(out, "--")?;
        }
        write!(out, "{}", crate::info::render(&port, spec, &options)?)?;
    }

    Ok(())
}
