use crate::{index::Index, query::Matcher, tcl_list};
use anyhow::Result;
use clap::Args;
use std::io::Write;

#[derive(Args)]
pub struct Search {
    #[arg(long, conflicts_with_all = ["glob", "regex"])]
    exact: bool,
    #[arg(long, conflicts_with = "regex")]
    glob: bool,
    #[arg(long)]
    regex: bool,
    #[arg(long)]
    case_sensitive: bool,
    #[command(flatten)]
    fields: crate::fields::SearchFields,
    #[arg(long)]
    line: bool,
    #[arg(required = true)]
    patterns: Vec<String>,
}

pub(super) fn run(out: &mut impl Write, index: &Index, s: Search, quiet: bool) -> Result<()> {
    let fields = if s.fields.ordered.is_empty() {
        vec!["name".into(), "description".into()]
    } else {
        s.fields.ordered
    };
    for (pattern_index, p) in s.patterns.iter().enumerate() {
        if pattern_index > 0 {
            writeln!(out, "--")?;
        }
        let pattern = if !s.exact && !s.regex && !s.glob && !p.contains(['*', '?']) {
            format!("*{p}*")
        } else {
            p.clone()
        };
        let matcher = Matcher::new(&pattern, s.exact, s.regex, s.case_sensitive)?;
        let mut matches: Vec<_> = index
            .ports()
            .values()
            .filter(|port| {
                fields.iter().any(|field| {
                    port.fields
                        .get(crate::fields::canonical(field))
                        .is_some_and(|value| matcher.matches(value))
                })
            })
            .collect();
        matches.sort_by(|a, b| crate::order::dictionary(&a.name, &b.name));
        for (i, port) in matches.iter().enumerate() {
            if quiet {
                writeln!(out, "{}", port.name)?;
            } else if s.line {
                writeln!(
                    out,
                    "{}\t{}\t{}\t{}",
                    port.name,
                    port.get("version"),
                    port.get("categories"),
                    port.get("description")
                )?;
            } else {
                if i > 0 {
                    writeln!(out)?;
                }
                let revision = port.get("revision");
                let version = if revision.is_empty() || revision == "0" {
                    port.get("version").to_string()
                } else {
                    port.version()
                };
                let categories = tcl_list::parse(port.get("categories"))?.join(", ");
                write!(out, "{} @{}", port.name, version)?;
                if !categories.is_empty() {
                    write!(out, " ({categories})")?;
                }
                writeln!(out, "\n    {}", port.get("description"))?;
            }
        }
        if !quiet {
            if matches.is_empty() {
                writeln!(out, "No match for {p} found")?;
            } else if matches.len() > 1 && !s.line {
                writeln!(out, "\nFound {} ports.", matches.len())?;
            }
        }
    }

    Ok(())
}
