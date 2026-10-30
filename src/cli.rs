mod actions;

use clap::Parser;
use port_rs::commands::Command;
use std::{ffi::OsString, path::PathBuf};

#[derive(Parser)]
#[command(
    name = "port",
    version,
    args_override_self = true,
    styles = port_rs::theme::help_styles(),
    about = "Standalone Rust port CLI (early implementation; no Tcl required)"
)]
pub(crate) struct Cli {
    #[arg(long = "index-file", global = true, env = "PORT_RS_INDEX")]
    pub(crate) index_file: Option<PathBuf>,
    /// Read an existing MacPorts registry at this path
    #[arg(long, global = true, env = "PORT_RS_REGISTRY")]
    pub(crate) registry_file: Option<PathBuf>,
    #[arg(short, long, global = true)]
    pub(crate) quiet: bool,
    /// Include full dependency specifications in metadata output
    #[arg(short, long, global = true)]
    pub(crate) verbose: bool,
    #[command(subcommand)]
    pub(crate) command: Command,
}
/// Global options before the first action form the defaults for every action.
/// Only an entire argument equal to `;` separates commands; no shell is involved.
pub(crate) fn invocations(args: impl IntoIterator<Item = OsString>) -> Vec<Vec<OsString>> {
    let mut args = args.into_iter();
    let program = args.next().unwrap_or_else(|| "port".into());
    let args: Vec<_> = args.collect();
    let end = leading_options(&args);
    let base: Vec<_> = std::iter::once(program)
        .chain(args[..end].iter().cloned())
        .collect();
    if end == args.len() {
        return vec![base]; // Let clap handle missing actions, help, and version.
    }
    args[end..]
        .split(|arg| arg == ";")
        .filter(|command| !command.is_empty())
        .map(|command| base.iter().chain(command).cloned().collect())
        .collect()
}

fn leading_options(args: &[OsString]) -> usize {
    let mut end = 0;
    while end < args.len() {
        if args[end] == "--index-file" || args[end] == "--registry-file" {
            end = (end + 2).min(args.len());
        } else if args[end].to_string_lossy().starts_with('-') && args[end] != "--" {
            end += 1;
        } else {
            break;
        }
    }
    end
}

impl Cli {
    pub(crate) fn parse_invocation(mut args: Vec<OsString>) -> anyhow::Result<Self> {
        let action = 1 + leading_options(&args[1..]);
        if let Some(name) = args.get(action).and_then(|name| name.to_str()) {
            let expanded = actions::expand(name)?;
            let help = expanded == "help";
            args[action] = expanded.into();
            if help && let Some(name) = args.get(action + 1).and_then(|name| name.to_str()) {
                args[action + 1] = actions::expand(name)?.into();
            }
        }
        Ok(Self::parse_from(args))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    #[test]
    fn help_uses_rocksky_palette_including_subcommands() {
        let mut command = Cli::command();
        command.build();
        let help = command.render_long_help();
        assert!(help.ansi().to_string().contains("\x1b[38;2;0;232;198m"));
        assert!(help.ansi().to_string().contains("\x1b[38;2;0;198;232m"));
        assert!(help.ansi().to_string().contains("\x1b[38;2;130;100;255m"));
        assert!(!help.to_string().contains('\x1b'));
        for subcommand in command.get_subcommands_mut() {
            let help = subcommand.render_long_help().ansi().to_string();
            assert!(
                help.contains("\x1b[38;2;0;232;198m"),
                "{}",
                subcommand.get_name()
            );
        }
    }
}
