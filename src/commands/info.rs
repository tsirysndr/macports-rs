use crate::{index::Index, query};
use anyhow::{Result, ensure};
use clap::Args;
use std::io::Write;

#[derive(Args)]
pub struct Info {
    /// Read metadata from PortIndex (currently the only supported info source)
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
) -> Result<()> {
    let specs = query::specifications(index, &s.ports)?;
    ensure!(
        s.index || specs.iter().all(|spec| !spec.has_modifiers()),
        "evaluating version/variant/option-specific metadata requires the Portfile engine; use info --index for available indexed metadata"
    );
    let ports = specs
        .iter()
        .map(|spec| query::lookup(index, &spec.name))
        .collect::<Result<Vec<_>>>()?;
    let options = crate::info::Options {
        fields: &s.fields.ordered,
        line: s.line,
        pretty: s.pretty,
        quiet,
        verbose,
        width: usize::from(console::Term::stdout().size().1).max(40),
    };
    for (i, (spec, port)) in specs.iter().zip(ports).enumerate() {
        if i > 0 && !s.line {
            writeln!(out, "--")?;
        }
        write!(out, "{}", crate::info::render(port, spec, &options)?)?;
    }

    Ok(())
}
