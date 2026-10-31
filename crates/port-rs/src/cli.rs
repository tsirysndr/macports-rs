mod actions;

use clap::Parser;
use port_rs::commands::Command;
use std::{ffi::OsString, path::PathBuf};

#[derive(Parser)]
#[command(
    name = "port",
    version,
    args_override_self = true,
    disable_help_subcommand = true,
    styles = port_rs::theme::help_styles(),
    about = "Standalone Rust implementation of the MacPorts port CLI"
)]
pub(crate) struct Cli {
    #[arg(long = "index-file", global = true, env = "PORT_RS_INDEX")]
    pub(crate) index_file: Option<PathBuf>,
    /// Read an existing MacPorts registry at this path
    #[arg(long, global = true, env = "PORT_RS_REGISTRY")]
    pub(crate) registry_file: Option<PathBuf>,
    #[arg(short, long, global = true)]
    pub(crate) quiet: bool,
    /// Verbose output: informational messages and full dependency specifications
    #[arg(short, long, global = true)]
    pub(crate) verbose: bool,
    /// Preview install operations without modifying the registry or installing files
    #[arg(short = 'y', long, global = true)]
    pub(crate) dry_run: bool,
    /// Build from source only; never use binary archives
    #[arg(short = 's', long = "source-only", global = true)]
    pub(crate) source_only: bool,
    /// Install from binary archives only; never build from source
    #[arg(short = 'b', long = "binary-only", global = true)]
    pub(crate) binary_only: bool,
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

pub(crate) fn leading_count(args: &[OsString]) -> usize {
    leading_options(args)
}

fn leading_options(args: &[OsString]) -> usize {
    let mut end = 0;
    while end < args.len() {
        let arg = args[end].to_string_lossy();
        if arg == "--index-file" || arg == "--registry-file" {
            end = (end + 2).min(args.len());
        } else if arg.starts_with('-')
            && !arg.starts_with("--")
            && arg.len() > 1
            && (arg.ends_with('D') || arg.ends_with('F'))
        {
            // -D portdir and -F cmdfile take the next argument.
            end = (end + 2).min(args.len());
        } else if arg.starts_with('-') && arg != "--" {
            end += 1;
        } else {
            break;
        }
    }
    end
}

/// Upstream's global options that clap does not parse, from the options
/// before the action. They are removed from `args`; the rest stay for clap.
#[derive(Default)]
pub(crate) struct Leading {
    pub(crate) options: port_rs::options::Global,
    pub(crate) directory: Option<String>,
    pub(crate) command_files: Vec<String>,
}

pub(crate) fn take_leading(args: &mut Vec<OsString>) -> anyhow::Result<Leading> {
    let mut leading = Leading::default();
    let end = 1 + leading_options(&args[1..]);
    let mut kept = vec![args[0].clone()];
    let mut index = 1;
    while index < end {
        let arg = args[index].to_string_lossy().into_owned();
        index += 1;
        if arg.starts_with("--") || !arg.starts_with('-') {
            kept.push(arg.into());
            if matches!(
                args[index - 1].to_str(),
                Some("--index-file" | "--registry-file")
            ) && index < end
            {
                kept.push(args[index].clone());
                index += 1;
            }
            continue;
        }
        let mut clap_letters = String::new();
        for letter in arg[1..].chars() {
            let options = &mut leading.options;
            match letter {
                'q' => {
                    options.noninteractive = true;
                    clap_letters.push(letter);
                }
                'v' => {
                    options.verbose = true;
                    clap_letters.push(letter);
                }
                'y' => {
                    options.dry_run = true;
                    clap_letters.push(letter);
                }
                's' | 'b' => clap_letters.push(letter),
                'd' => {
                    options.debug = true;
                    options.verbose = true;
                }
                'N' => options.noninteractive = true,
                'p' => options.process_all = true,
                'f' => options.force = true,
                'o' => options.ignore_different = true,
                'n' => options.nodeps = true,
                'u' => options.uninstall_old = true,
                'R' => options.dependents = true,
                'c' => options.autoclean = Some(true),
                'k' => options.autoclean = Some(false),
                't' => options.trace = true,
                'D' | 'F' => {
                    let value = args
                        .get(index)
                        .filter(|_| index < end)
                        .map(|value| value.to_string_lossy().into_owned())
                        .ok_or_else(|| anyhow::anyhow!("-{letter} requires an argument"))?;
                    index += 1;
                    if letter == 'D' {
                        leading.directory = Some(value);
                    } else {
                        leading.command_files.push(value);
                    }
                }
                other => anyhow::bail!("unknown option -{other}"),
            }
        }
        if !clap_letters.is_empty() {
            kept.push(format!("-{clap_letters}").into());
        }
    }
    kept.extend(args.drain(end..));
    *args = kept;
    Ok(leading)
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
        let mut cli = Self::parse_from(args);
        if let Command::Install(install) = &mut cli.command {
            install.dry_run = cli.dry_run;
        } else if let Command::Reclaim(reclaim) = &mut cli.command {
            reclaim.dry_run = cli.dry_run;
        } else if let Command::RevUpgrade(revupgrade) = &mut cli.command {
            revupgrade.dry_run = cli.dry_run;
        }
        Ok(cli)
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
