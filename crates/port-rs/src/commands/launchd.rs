//! `port load`, `unload` and `reload` (portload, portunload, portreload):
//! run launchctl for the startup items a port declares.
use crate::{index::Index, query};
use anyhow::{Result, ensure};
use std::path::Path;

pub(super) fn run(
    index: &Index,
    ports: &[String],
    action: &str,
    quiet: bool,
    root: &Path,
) -> Result<()> {
    let host = super::local::Host::new(root, None, quiet)?;
    let worker =
        crate::install::mirror_worker::MirrorWorker::new(host.context.mirror_timings.clone(), 8)?;
    let targets: &[&str] = match action {
        "reload" => &["unload", "load"],
        "unload" => &["unload"],
        _ => &["load"],
    };
    for mut spec in query::specifications(index, ports)? {
        host.context.variants.apply(&mut spec);
        let port = query::lookup(index, &spec.name)?;
        let mut session = host.open(root, port, &spec)?;
        super::phase::prepare(&mut session, &host, &worker, &spec.name)?;
        host.progress.close_log();
        for target in targets {
            let failed = session.invoke("eval_targets", &[target])?;
            ensure!(failed != "1", "Failed to {target} {}", spec.name);
        }
    }
    Ok(())
}
