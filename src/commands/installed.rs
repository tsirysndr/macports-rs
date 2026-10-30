use crate::registry::{self, Entry};
use anyhow::{Context, Result, ensure};
use clap::Args;
use std::{io::Write, path::Path};

#[derive(Args)]
pub struct Installed {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}

pub(super) fn run(
    out: &mut impl Write,
    path: Option<&Path>,
    args: Installed,
    quiet: bool,
    verbose: bool,
) -> Result<()> {
    let registry = registry::Registry::open(path)?;
    let entries = registry
        .as_ref()
        .map(registry::Registry::entries)
        .transpose()?
        .unwrap_or_default();
    let selected = registry::expression::select(
        &entries,
        &args.ports,
        registry::expression::Missing::Error,
        registry.as_ref(),
    )?;
    let lines = selected
        .into_iter()
        .map(|entry| render(entry, verbose))
        .collect::<Result<Vec<_>>>()?;
    if lines.is_empty() {
        if !quiet {
            writeln!(
                out,
                "{}",
                if args.ports.is_empty() {
                    "No ports are installed."
                } else {
                    "None of the specified ports are installed."
                }
            )?;
        }
    } else {
        if !quiet {
            writeln!(out, "The following ports are currently installed:")?;
        }
        for line in lines {
            writeln!(out, "{line}")?;
        }
    }
    Ok(())
}
fn render(entry: &Entry, verbose: bool) -> Result<String> {
    let mut line = format!(
        "  {} @{}{}",
        entry.name,
        entry.full_version(),
        if entry.active { " (active)" } else { "" }
    );
    if verbose {
        if !entry.requested_variants.is_empty() && entry.requested_variants != "0" {
            line.push_str(&format!(
                " requested_variants='{}'",
                entry.requested_variants
            ));
        }
        if !entry.platform.is_empty()
            && entry.platform != "0"
            && !entry.os_major.is_empty()
            && entry.os_major != "0"
        {
            line.push_str(&format!(
                " platform='{} {}'",
                entry.platform, entry.os_major
            ));
        }
        if !entry.archs.is_empty() && entry.archs != "0" {
            line.push_str(&format!(" archs='{}'", entry.archs));
        }
        if !entry.date.is_empty() {
            line.push_str(&format!(" date='{}'", date(&entry.date)?));
        }
    }
    Ok(line)
}
#[cfg(target_os = "macos")]
fn date(value: &str) -> Result<String> {
    let timestamp: libc::time_t = value
        .parse()
        .context("invalid registry installation timestamp")?;
    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
    let mut buffer = [0 as libc::c_char; 64];
    // SAFETY: valid timestamp and writable tm/buffer storage; localtime_r initializes
    // tm on success, and strftime is bounded by the provided buffer length.
    unsafe {
        ensure!(
            !libc::localtime_r(&timestamp, local.as_mut_ptr()).is_null(),
            "registry timestamp out of range"
        );
        let len = libc::strftime(
            buffer.as_mut_ptr(),
            buffer.len(),
            c"%Y-%m-%dT%H:%M:%S%z".as_ptr(),
            local.as_ptr(),
        );
        ensure!(len > 0, "cannot format registry timestamp");
        Ok(std::ffi::CStr::from_ptr(buffer.as_ptr()).to_str()?.into())
    }
}
#[cfg(not(target_os = "macos"))]
fn date(_value: &str) -> Result<String> {
    anyhow::bail!("verbose registry timestamp formatting requires macOS")
}
