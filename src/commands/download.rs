use crate::download::{self, Manifest};
use anyhow::{Context, Result};
use clap::Args;
use std::{io::Write, path::PathBuf};

#[derive(Args)]
pub struct Download {
    #[arg(long)]
    manifest: PathBuf,
    #[arg(short = 'o', long, default_value = ".")]
    output: PathBuf,
    #[arg(short = 'j', long, default_value_t = 8, value_parser = clap::value_parser!(u16).range(1..=128))]
    jobs: u16,
}
pub(super) fn run(out: &mut impl Write, args: Download, quiet: bool) -> Result<()> {
    let manifest: Manifest = serde_json::from_slice(
        &std::fs::read(&args.manifest).context("cannot read download manifest")?,
    )?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    for result in runtime.block_on(download::run_with_progress(
        &manifest,
        &args.output,
        usize::from(args.jobs),
        &crate::progress::Progress::new(quiet),
    ))? {
        if !quiet {
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

    Ok(())
}
