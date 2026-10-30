use crate::{
    index::{Index, Port},
    output, query,
    spec::PortSpec,
    tcl_list,
};
use anyhow::{Context, Result};
use clap::Args;
use std::{fmt::Write as _, io::Write, path::Path};

#[derive(Args)]
pub struct Variants {
    /// Read variant metadata from PortIndex without evaluating Portfiles
    #[arg(long)]
    pub(super) index: bool,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}

pub(super) fn run(
    out: &mut impl Write,
    index: &Index,
    args: Variants,
    quiet: bool,
    root: Option<&Path>,
) -> Result<()> {
    let specs = query::specifications(index, &args.ports)?;
    let variables = super::local::variables(root)?;
    let width = usize::from(console::Term::stdout().size().1).max(40);
    let rendered = specs
        .iter()
        .map(|spec| {
            let port = query::lookup(index, &spec.name)?;
            let evaluated = root
                .map(|root| crate::portfile::evaluate_from_tree(root, port, &variables, spec))
                .transpose()?;
            let port = evaluated.as_ref().map(|value| &value.port).unwrap_or(port);
            render(port, spec, quiet, width)
                .with_context(|| format!("{}: variant metadata", port.name))
        })
        .collect::<Result<Vec<_>>>()?;
    for text in rendered {
        write!(out, "{text}")?;
    }
    Ok(())
}

fn render(port: &Port, spec: &PortSpec, quiet: bool, width: usize) -> Result<String> {
    let mut text = String::new();
    let Some(variants) = port.fields.get("variants") else {
        if !quiet {
            writeln!(text, "{} has no variants", port.name)?;
        }
        return Ok(text);
    };
    if !quiet {
        writeln!(text, "{} has the variants:", port.name)?;
    }
    let mut variants = tcl_list::parse(variants)?;
    variants.sort(); // Upstream uses ASCII order here, not dictionary order.
    let metadata = tcl_list::dictionary(port.get("vinfo")).context("vinfo")?;
    for name in variants {
        let mut marker = "   ".to_owned();
        let details = metadata
            .get(&name)
            .map(|raw| tcl_list::dictionary(raw))
            .transpose()
            .with_context(|| format!("vinfo {name}"))?;
        if let Some(details) = &details {
            if let Some(enabled) = spec.variants.get(&name) {
                marker = format!("  {}", if *enabled { '+' } else { '-' });
            } else if let Some(default) = details.get("is_default") {
                marker = format!("[{default}]");
            }
        }
        if let Some(description) = details
            .as_ref()
            .and_then(|details| details.get("description"))
        {
            let prefix = format!("{marker}{name}: ");
            writeln!(
                text,
                "{}",
                output::wrap(description.trim(), &prefix, 5 + name.chars().count(), width)
            )?;
        } else {
            writeln!(text, "{marker}{name}")?;
        }
        if let Some(details) = details {
            for (field, label) in [("conflicts", "conflicts with"), ("requires", "requires")] {
                if let Some(value) = details.get(field) {
                    writeln!(text, "     * {label} {}", value.trim())?;
                }
            }
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descriptions_wrap_below_the_variant_label() {
        let port = Port {
            name: "example".into(),
            fields: [
                ("variants".into(), "ssl".into()),
                (
                    "vinfo".into(),
                    "ssl {description {Enable secure networking support}}".into(),
                ),
            ]
            .into_iter()
            .collect(),
        };
        assert_eq!(
            render(&port, &PortSpec::named("example"), true, 25).unwrap(),
            "   ssl: Enable secure\n        networking\n        support\n"
        );
    }
}
