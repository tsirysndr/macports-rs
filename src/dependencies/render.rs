use super::{Filter, Graph, in_field};
use crate::{index::Port, output};
use anyhow::Result;
use std::io::Write;

#[derive(Clone, Copy)]
pub struct OutputOptions {
    pub quiet: bool,
    pub verbose: bool,
    pub width: usize,
}
fn full_name(port: &Port) -> String {
    format!(
        "{} @{}{}",
        port.name,
        port.version(),
        port.get("canonical_active_variants")
    )
}

/// Prepared direct output; all dependency lists are parsed before writing begins.
pub struct Direct<'a> {
    port: &'a Port,
    lines: Vec<String>,
    count: usize,
    quiet: bool,
}
impl<'a> Direct<'a> {
    pub fn prepare(port: &'a Port, filter: Filter, options: OutputOptions) -> Result<Self> {
        let mut lines = Vec::new();
        let mut count = 0;
        for field in filter.fields() {
            if !port.fields.contains_key(field) {
                continue;
            }
            let dependencies = in_field(port, field)?;
            count += dependencies.len();
            let label = match field {
                "depends_fetch" => "Fetch",
                "depends_extract" => "Extract",
                "depends_patch" => "Patch",
                "depends_build" => "Build",
                "depends_lib" => "Library",
                "depends_run" => "Runtime",
                "depends_test" => "Test",
                _ => unreachable!(),
            };
            let values = dependencies
                .iter()
                .map(|d| d.display(options.verbose))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(output::wrap(
                &values,
                &format!("{:<22}", format!("{label} Dependencies:")),
                22,
                options.width,
            ));
        }
        Ok(Self {
            port,
            lines,
            count,
            quiet: options.quiet,
        })
    }
    pub fn write(&self, out: &mut impl Write) -> Result<()> {
        let Self {
            port,
            lines,
            count,
            quiet,
        } = self;
        if *count == 0 {
            if !quiet {
                writeln!(out, "{} has no dependencies.", full_name(port))?;
            }
        } else {
            if !quiet {
                writeln!(out, "Full Name: {}", full_name(port))?;
            }
            writeln!(out, "{}", lines.join("\n"))?;
        }
        Ok(())
    }
}

pub fn direct(
    out: &mut impl Write,
    port: &Port,
    filter: Filter,
    options: OutputOptions,
) -> Result<()> {
    Direct::prepare(port, filter, options)?.write(out)
}

pub fn recursive(
    out: &mut impl Write,
    graph: &Graph<'_>,
    full: bool,
    options: OutputOptions,
) -> Result<()> {
    if !options.quiet {
        if graph.is_empty() {
            writeln!(out, "{} has no dependencies.", full_name(graph.root))?;
        } else {
            writeln!(
                out,
                "The following ports are dependencies of {}:",
                full_name(graph.root)
            )?;
        }
    }
    graph.walk(full, |dependency, depth, cyclic| {
        writeln!(
            out,
            "{:indent$}{}{}",
            "",
            dependency.display(options.verbose),
            if cyclic { " (cyclic dependency)" } else { "" },
            indent = depth * 2
        )?;
        Ok(())
    })
}
