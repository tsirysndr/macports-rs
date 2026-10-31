//! `port mirror` (portmirror): fetch and verify each port's distfiles and
//! record them as mirrored. The record is a plain-text file beside upstream's
//! binary `distfiles_mirror.db`, which is left untouched.
use crate::{index::Index, query};
use anyhow::Result;
use clap::Args;
use std::{collections::BTreeSet, fs, path::Path};

#[derive(Args)]
pub struct Mirror {
    /// Start a new mirror record
    #[arg(long)]
    new: bool,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}

const RECORD: &str = "distfiles_mirror.txt";

pub(super) fn run(index: &Index, args: Mirror, quiet: bool, root: &Path) -> Result<()> {
    let host = super::local::Host::new(root, None, quiet)?;
    let worker =
        crate::install::mirror_worker::MirrorWorker::new(host.context.mirror_timings.clone(), 8)?;
    let portdbpath = host
        .context
        .variables
        .get("portdbpath")
        .cloned()
        .unwrap_or_default();
    let record = Path::new(&portdbpath).join(RECORD);
    if args.new {
        let _ = fs::remove_file(&record);
    }
    let mut mirrored: BTreeSet<String> = fs::read_to_string(&record)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    for mut spec in query::specifications(index, &args.ports)? {
        host.context.variants.apply(&mut spec);
        let port = query::lookup(index, &spec.name)?;
        let mut session = host.open(root, port, &spec)?;
        let subport = session.scalar("subport")?;
        let license = crate::tcl_list::parse(&session.scalar("license").unwrap_or_default())
            .unwrap_or_default();
        if license
            .iter()
            .any(|item| item.eq_ignore_ascii_case("nomirror"))
        {
            host.progress.log(
                "info",
                &format!("Not mirroring {subport} due to license NoMirror"),
            );
            continue;
        }
        if session.scalar("fetch.type").unwrap_or_default() != "standard" {
            continue;
        }
        let label = super::phase::prepare(&mut session, &host, &worker, &spec.name)?;
        super::phase::run_targets(&mut session, &label, &["main", "fetch"])?;
        let distpath = std::path::PathBuf::from(session.scalar("distpath")?);
        let files: Vec<String> = crate::tcl_list::parse(&session.invoke(
            "uplevel",
            &[
                "#0",
                "set r {}; foreach {t f} $portfetch::fetch_urls {lappend r $f}; set r",
            ],
        )?)?;
        if let Err(err) = super::phase::run_targets(&mut session, &label, &["checksum"]) {
            for file in &files {
                let _ = fs::remove_file(distpath.join(file));
            }
            anyhow::bail!("portmirror: checksum failed: {err:#}");
        }
        for file in files {
            let path = distpath.join(file);
            if path.is_file() {
                mirrored.insert(path.to_string_lossy().into_owned());
            }
        }
        host.progress.close_log();
    }
    if let Some(parent) = record.parent() {
        fs::create_dir_all(parent)?;
    }
    let text: String = mirrored.into_iter().map(|line| line + "\n").collect();
    fs::write(&record, text)?;
    Ok(())
}
