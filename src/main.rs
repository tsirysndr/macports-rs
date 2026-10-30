use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Parser, Subcommand};
use port_rs::{
    download::{self, Manifest},
    index::Index,
    query::{self, Matcher},
    tcl_list,
};
use std::{
    io::{self, Write},
    path::PathBuf,
};

#[derive(Parser)]
#[command(
    name = "port",
    version,
    styles = port_rs::theme::help_styles(),
    about = "Standalone Rust port CLI (early implementation; no Tcl required)"
)]
struct Cli {
    #[arg(long = "index-file", global = true, env = "PORT_RS_INDEX")]
    index_file: Option<PathBuf>,
    #[arg(short, long, global = true)]
    quiet: bool,
    /// Include full dependency specifications in metadata output
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// List ports from a local MacPorts PortIndex
    List {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        ports: Vec<String>,
    },
    /// Search names and descriptions (case insensitive by default)
    Search(Search),
    /// Display indexed port metadata
    Info(Info),
    /// Print selected port names
    Echo(Selection),
    /// Display indexed dependencies (does not evaluate variants)
    Deps(Selection),
    /// Display indexed variants
    Variants(Selection),
    /// Download files concurrently from a checksummed JSON manifest (Rust extension)
    Download {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(short = 'o', long, default_value = ".")]
        output: PathBuf,
        #[arg(short = 'j', long, default_value_t = 8, value_parser = clap::value_parser!(u16).range(1..=128))]
        jobs: u16,
    },
    #[command(external_subcommand)]
    Unsupported(Vec<String>),
}
#[derive(Args)]
struct Selection {
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}
#[derive(Args)]
struct Search {
    #[arg(long, conflicts_with_all = ["glob", "regex"])]
    exact: bool,
    #[arg(long, conflicts_with = "regex")]
    glob: bool,
    #[arg(long)]
    regex: bool,
    #[arg(long)]
    case_sensitive: bool,
    #[command(flatten)]
    fields: port_rs::fields::SearchFields,
    #[arg(long)]
    line: bool,
    #[arg(required = true)]
    patterns: Vec<String>,
}
#[derive(Args)]
struct Info {
    /// Read metadata from PortIndex (currently the only supported info source)
    #[arg(long)]
    index: bool,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
    #[command(flatten)]
    fields: port_rs::fields::InfoFields,
    /// Tab-separated output in the requested field order
    #[arg(long)]
    line: bool,
    /// Format fields as a human-readable summary
    #[arg(long)]
    pretty: bool,
}
fn main() {
    if let Err(error) = run(Cli::parse()) {
        if error
            .downcast_ref::<io::Error>()
            .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
        {
            return;
        }
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}
fn run(cli: Cli) -> Result<()> {
    // Read-only commands avoid starting an async runtime or constructing an HTTP client.
    if let Command::Unsupported(args) = &cli.command {
        bail!(
            "command {:?} is not implemented in the standalone Rust engine; no Tcl or MacPorts fallback is used",
            args[0]
        );
    }
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    if let Command::Download {
        manifest,
        output,
        jobs,
    } = &cli.command
    {
        let manifest: Manifest = serde_json::from_slice(
            &std::fs::read(manifest).context("cannot read download manifest")?,
        )?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        for result in runtime.block_on(download::run_with_progress(
            &manifest,
            output,
            usize::from(*jobs),
            &port_rs::progress::Progress::new(cli.quiet),
        ))? {
            if !cli.quiet {
                writeln!(
                    out,
                    "{} {}",
                    if result.cached {
                        "Verified"
                    } else {
                        "Downloaded"
                    },
                    result.path.display()
                )?;
            }
        }
        out.flush()?;
        return Ok(());
    }
    let index = port_rs::source::load(cli.index_file.as_deref(), cli.quiet)?;
    let action = match &cli.command {
        Command::List { .. } => "list",
        Command::Echo(_) => "echo",
        Command::Deps(_) => "deps",
        Command::Variants(_) => "variants",
        _ => "",
    };
    match cli.command {
        Command::List { ports } => {
            let ports = if ports.is_empty() {
                vec!["all".into()]
            } else {
                ports
            };
            render_selection(&mut out, &index, &ports, cli.quiet, "list")?;
        }
        Command::Echo(s) | Command::Deps(s) | Command::Variants(s) => {
            render_selection(&mut out, &index, &s.ports, cli.quiet, action)?;
        }
        Command::Search(s) => {
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
                let matches: Vec<_> = index
                    .ports
                    .values()
                    .filter(|port| {
                        fields.iter().any(|field| {
                            port.fields
                                .get(port_rs::fields::canonical(field))
                                .is_some_and(|value| matcher.matches(value))
                        })
                    })
                    .collect();
                for (i, port) in matches.iter().enumerate() {
                    if cli.quiet {
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
                if !cli.quiet {
                    if matches.is_empty() {
                        writeln!(out, "No match for {p} found")?;
                    } else if matches.len() > 1 && !s.line {
                        writeln!(out, "\nFound {} ports.", matches.len())?;
                    }
                }
            }
        }
        Command::Info(s) => {
            let specs = query::specifications(&index, &s.ports)?;
            ensure!(
                s.index || specs.iter().all(|spec| !spec.has_modifiers()),
                "evaluating version/variant/option-specific metadata requires the Portfile engine; use info --index for available indexed metadata"
            );
            let ports = specs
                .iter()
                .map(|spec| query::lookup(&index, &spec.name))
                .collect::<Result<Vec<_>>>()?;
            let options = port_rs::info::Options {
                fields: &s.fields.ordered,
                line: s.line,
                pretty: s.pretty,
                quiet: cli.quiet,
                verbose: cli.verbose,
                width: usize::from(console::Term::stdout().size().1).max(40),
            };
            for (i, (spec, port)) in specs.iter().zip(ports).enumerate() {
                if i > 0 && !s.line {
                    writeln!(out, "--")?;
                }
                write!(out, "{}", port_rs::info::render(port, spec, &options)?)?;
            }
        }
        Command::Download { .. } | Command::Unsupported(_) => unreachable!(),
    }
    out.flush()?;
    Ok(())
}
fn render_selection(
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
    if matches!(action, "deps" | "variants") {
        ensure!(
            specs.iter().all(|spec| !spec.has_modifiers()),
            "{action} with version, variant, or option overrides requires Portfile evaluation, which is not implemented"
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
            "variants" => writeln!(
                out,
                "{} has the variants: {}",
                port.name,
                tcl_list::parse(port.get("variants"))?.join(", ")
            )?,
            "deps" => {
                writeln!(out, "{} @{}", port.name, port.version())?;
                for field in [
                    "depends_fetch",
                    "depends_extract",
                    "depends_patch",
                    "depends_build",
                    "depends_lib",
                    "depends_run",
                    "depends_test",
                ] {
                    let dependencies = tcl_list::parse(port.get(field))?;
                    if !dependencies.is_empty() {
                        writeln!(out, "{field}: {}", dependencies.join(", "))?;
                    }
                }
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    #[test]
    fn help_uses_exact_accent_including_subcommands() {
        let mut command = Cli::command();
        command.build();
        let help = command.render_long_help();
        assert!(help.ansi().to_string().contains("\x1b[38;2;117;23;252m"));
        assert!(!help.to_string().contains('\x1b'));
        for subcommand in command.get_subcommands_mut() {
            let help = subcommand.render_long_help().ansi().to_string();
            assert!(
                help.contains("\x1b[38;2;117;23;252m"),
                "{}",
                subcommand.get_name()
            );
        }
    }
}
