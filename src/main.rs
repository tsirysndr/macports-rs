use clap::Parser;
use port_rs::commands::{self, Command};
use std::{io, path::PathBuf};

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
fn main() {
    if let Err(error) = {
        let cli = Cli::parse();
        commands::run(
            cli.command,
            cli.index_file.as_deref(),
            cli.quiet,
            cli.verbose,
        )
    } {
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
