use crate::registry::{Registry, path};
use anyhow::{Context, Result};
use clap::Args;
use std::{
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Args)]
pub struct Provides {
    #[arg(required = true)]
    files: Vec<PathBuf>,
}
pub(super) fn run(
    out: &mut impl Write,
    registry_file: Option<&Path>,
    args: Provides,
    quiet: bool,
) -> Result<()> {
    let registry = Registry::open(registry_file)?;
    let mut lines = Vec::new();
    for filename in args.files {
        let file = path::normalize(&filename)
            .with_context(|| format!("cannot normalize {}", filename.display()))?;
        let metadata = std::fs::symlink_metadata(&file);
        let line = match metadata {
            Err(_) => {
                if quiet {
                    None
                } else {
                    Some(format!("{} does not exist.", file.display()))
                }
            }
            Ok(metadata) if metadata.is_dir() => {
                if quiet {
                    None
                } else {
                    Some(format!("{} is a directory.", file.display()))
                }
            }
            Ok(_) => {
                let owner = registry
                    .as_ref()
                    .map(|registry| {
                        registry.owner(
                            file.to_str().context("registry paths must be UTF-8")?,
                            path::case_sensitive(&file),
                        )
                    })
                    .transpose()?
                    .flatten();
                match owner {
                    Some(owner) => Some(if quiet {
                        owner
                    } else {
                        format!("{} is provided by: {owner}", file.display())
                    }),
                    None if quiet => None,
                    None => Some(format!(
                        "{} is not provided by a MacPorts port.",
                        file.display()
                    )),
                }
            }
        };
        if let Some(line) = line {
            lines.push(line);
        }
    }
    for line in lines {
        writeln!(out, "{line}")?;
    }
    Ok(())
}
