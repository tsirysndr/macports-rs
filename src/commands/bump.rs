//! `port bump` (portbump): fetch distfiles and update the Portfile's checksums
//! to match them, resetting the revision.
use crate::{index::Index, install::source::checksum, query};
use anyhow::{Context, Result, bail};
use clap::Args;
use regex::Regex;
use std::{fs, io::Write, path::Path, process::Command};

#[derive(Args)]
pub struct Bump {
    /// Write Portfile.patch instead of changing the Portfile
    #[arg(long)]
    patch: bool,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}

fn mime_is_html(path: &Path) -> bool {
    Command::new("/usr/bin/file")
        .args(["--brief", "--mime"])
        .arg(path)
        .output()
        .is_ok_and(|output| {
            String::from_utf8_lossy(&output.stdout)
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim() == "text/html")
        })
}

pub(super) fn run(
    out: &mut impl Write,
    index: &Index,
    args: Bump,
    quiet: bool,
    root: &Path,
) -> Result<()> {
    let host = super::local::Host::new(root, None, quiet)?;
    let worker =
        crate::install::mirror_worker::MirrorWorker::new(host.context.mirror_timings.clone(), 8)?;
    let revision = Regex::new(r"(revision[\s\\]+)[0-9]+")?;
    for mut spec in query::specifications(index, &args.ports)? {
        host.context.variants.apply(&mut spec);
        let port = query::lookup(index, &spec.name)?;
        let mut session = host.open(root, port, &spec)?;
        let label = super::phase::prepare(&mut session, &host, &worker, &spec.name)?;
        super::phase::run_targets(&mut session, &label, &["main", "fetch"])?;
        host.progress.close_log();
        let portfile = root.join(port.get("portdir")).join("Portfile");
        let Ok(distfiles) = session.scalar("all_dist_files") else {
            continue;
        };
        let distfiles = crate::tcl_list::parse(&distfiles)?;
        let words = crate::tcl_list::parse(&session.scalar("checksums").unwrap_or_default())?;
        let table = checksum::parse(&words, &distfiles);
        let distpath = std::path::PathBuf::from(session.scalar("distpath")?);
        let mut mismatch = false;
        let mut html = None;
        let mut both: Vec<(String, String, String)> = Vec::new();
        for distfile in &distfiles {
            let path = distpath.join(distfile);
            if !path.is_file() {
                bail!("{distfile} does not exist in {}", distpath.display());
            }
            let Some(expected) = table.get(distfile).filter(|values| !values.is_empty()) else {
                host.progress
                    .error(&format!("No checksum set for {distfile}"));
                mismatch = true;
                continue;
            };
            let kinds: Vec<checksum::Kind> = expected.iter().map(|(kind, _)| *kind).collect();
            let calculated = checksum::calculate(&path, &kinds)?;
            for ((kind, sum), new) in expected.iter().zip(calculated) {
                if *sum != new {
                    host.progress.log(
                        "info",
                        &format!("Portfile bump: {distfile} {} {sum}", kind.name()),
                    );
                    mismatch = true;
                }
                both.push((kind.name().to_string(), sum.clone(), new));
            }
            let is_html_name = distfile.ends_with(".html") || distfile.ends_with(".htm");
            if !is_html_name && mime_is_html(&path) {
                let moved = distpath.join(format!("{distfile}.html"));
                fs::rename(&path, &moved)?;
                html = Some(moved);
            }
        }
        if !mismatch {
            writeln!(out, "No changes needed.")?;
            continue;
        }
        if let Some(moved) = html {
            host.progress.message("***");
            host.progress.message(
                "The non-matching file appears to be HTML. See this page for possible reasons",
            );
            host.progress.message("for the bump mismatch:");
            host.progress
                .message("<https://trac.macports.org/wiki/MisbehavingServers>");
            host.progress.message("***");
            host.progress
                .message(&format!("The file has been moved to: {}", moved.display()));
            bail!("Unable to verify file checksums");
        }
        if both.is_empty() {
            bail!("No checksums found in Portfile");
        }
        writeln!(out, "We will bump these:")?;
        for (kind, sum, new) in &both {
            writeln!(out, "Old {:<8} {sum}", format!("{kind}:"))?;
            writeln!(out, "New {:<8} {new}", format!("{kind}:"))?;
        }
        let original = fs::read_to_string(&portfile)?;
        let mut text = original.clone();
        for (kind, sum, new) in &both {
            let pattern = Regex::new(&format!(
                r"({}[\s\\]+){}",
                regex::escape(kind),
                regex::escape(sum)
            ))?;
            text = pattern
                .replace_all(&text, format!("${{1}}{new}"))
                .into_owned();
        }
        text = revision.replace_all(&text, "${1}0").into_owned();
        let directory = portfile.parent().context("Portfile has no directory")?;
        if args.patch {
            let mut updated = tempfile::Builder::new()
                .prefix("Portfile.bump.")
                .tempfile_in(directory)?;
            updated.write_all(text.as_bytes())?;
            let diff = Command::new("/usr/bin/diff")
                .args(["-u", "--label", "old/Portfile", "--label", "new/Portfile"])
                .arg(&portfile)
                .arg(updated.path())
                .output()
                .context("cannot run diff")?;
            if diff.status.code() == Some(1) {
                let patch = directory.join("Portfile.patch");
                fs::write(&patch, &diff.stdout)?;
                writeln!(
                    out,
                    "Portfile.patch successfully created at {}",
                    patch.display()
                )?;
            } else {
                writeln!(out, "No changes needed.")?;
            }
        } else {
            let permissions = fs::metadata(&portfile)?.permissions();
            let mut updated = tempfile::Builder::new()
                .prefix("Portfile.bump.")
                .tempfile_in(directory)?;
            updated.write_all(text.as_bytes())?;
            updated
                .persist(&portfile)
                .map_err(|err| anyhow::anyhow!("bump overwrite failed: {err}"))?;
            fs::set_permissions(&portfile, permissions)?;
            let subport = session.scalar("subport")?;
            let version = session.scalar("version")?;
            writeln!(
                out,
                "Checksums successfully bumped. Suggested commit message:"
            )?;
            writeln!(out, "{:<8}{subport}: update to {version}", "")?;
        }
    }
    Ok(())
}
