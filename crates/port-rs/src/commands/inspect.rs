use crate::{index::Index, query};
use anyhow::Result;
use clap::Args;
use std::io::Write;

#[derive(Args)]
pub struct Selection {
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    pub(super) ports: Vec<String>,
}
pub(super) fn run(
    out: &mut impl Write,
    index: &Index,
    selectors: &[String],
    quiet: bool,
    action: &str,
) -> Result<()> {
    let specs = query::specifications(index, selectors)?;
    if action == "list" && specs.iter().any(|spec| spec.version.is_some()) {
        eprintln!(
            "Warning: The 'list' action only shows the currently available version of each port. To see installed versions, use the 'installed' action."
        );
    }
    // Resolve the entire selection before producing output on lookup-based actions.
    let ports = if action == "echo" {
        Vec::new()
    } else {
        specs
            .iter()
            .map(|spec| query::lookup(index, &spec.name))
            .collect::<Result<Vec<_>>>()?
    };
    for (i, spec) in specs.iter().enumerate() {
        if action == "echo" {
            if quiet {
                writeln!(out, "{}", spec.name)?;
            } else {
                let version = spec.composite_version();
                let version = if version.is_empty() {
                    String::new()
                } else {
                    format!("@{version}")
                };
                let options = spec
                    .options
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                writeln!(out, "{:<30} {} {}", spec.name, version, options)?;
            }
            continue;
        }
        let port = ports[i];
        match action {
            "list" => writeln!(
                out,
                "{:<30} @{:<14} {}",
                port.name,
                port.get("version"),
                port.get("portdir")
            )?,
            _ => unreachable!(),
        }
    }
    Ok(())
}
