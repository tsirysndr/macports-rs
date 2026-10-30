use anyhow::{Context, Result, bail};
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
    about = "Standalone Rust port CLI (early implementation; no Tcl required)"
)]
struct Cli {
    #[arg(
        long = "index-file",
        global = true,
        env = "PORT_RS_INDEX",
        default_value = "PortIndex"
    )]
    index_file: PathBuf,
    #[arg(short, long, global = true)]
    quiet: bool,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// List ports from a local MacPorts PortIndex
    List { ports: Vec<String> },
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
    #[arg(required = true)]
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
    #[arg(long)]
    name: bool,
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
    #[arg(required = true)]
    ports: Vec<String>,
    #[arg(long)]
    name: bool,
    #[arg(long)]
    version: bool,
    #[arg(long)]
    description: bool,
    #[arg(long)]
    homepage: bool,
    #[arg(long)]
    categories: bool,
    #[arg(long)]
    license: bool,
    #[arg(long)]
    maintainers: bool,
    #[arg(long)]
    variants: bool,
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
    let index = Index::read(&cli.index_file)?;
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
                        matcher.matches(&port.name)
                            || (!s.name && matcher.matches(port.get("description")))
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
            let fields = [
                (s.name, "name"),
                (s.version, "version"),
                (s.description, "description"),
                (s.homepage, "homepage"),
                (s.categories, "categories"),
                (s.license, "license"),
                (s.maintainers, "maintainers"),
                (s.variants, "variants"),
            ];
            for port in query::select(&index, &s.ports)? {
                if fields.iter().any(|(enabled, _)| *enabled) {
                    for (_, field) in fields.iter().filter(|(enabled, _)| *enabled) {
                        if cli.quiet {
                            writeln!(out, "{}", port.get(field))?;
                        } else {
                            writeln!(out, "{field}: {}", port.get(field))?;
                        }
                    }
                } else {
                    writeln!(
                        out,
                        "{} @{} ({})",
                        port.name,
                        port.version(),
                        port.get("categories")
                    )?;
                    for field in [
                        "description",
                        "long_description",
                        "homepage",
                        "license",
                        "maintainers",
                        "variants",
                    ] {
                        if !port.get(field).is_empty() {
                            writeln!(out, "{field}: {}", port.get(field))?;
                        }
                    }
                }
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
    for port in query::select(index, selectors)? {
        match action {
            "echo" if quiet => writeln!(out, "{}", port.name)?,
            "echo" => writeln!(out, "{:<30}  ", port.name)?,
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
