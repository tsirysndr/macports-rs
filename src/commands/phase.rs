use crate::{build, index::Index, query};
use anyhow::{Result, bail};
use clap::Args;
use std::path::Path;

#[derive(Args)]
pub struct Phase {
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}

const ORDER: [&str; 8] = [
    "main",
    "fetch",
    "checksum",
    "extract",
    "patch",
    "configure",
    "build",
    "destroot",
];

/// Run the requested source phase and its prerequisites for each port in order.
pub(super) fn run(
    index: &Index,
    registry_file: Option<&Path>,
    args: Phase,
    target: &str,
    quiet: bool,
    root: &Path,
) -> Result<()> {
    let Some(end) = ORDER.iter().position(|phase| *phase == target) else {
        bail!("unsupported phase {target}");
    };
    let phases = &ORDER[..=end];
    let mut specs = query::specifications(index, &args.ports)?;
    let host = super::local::Host::new(root, registry_file, quiet)?;
    let worker =
        crate::install::mirror_worker::MirrorWorker::new(host.context.mirror_timings.clone(), 8)?;
    for spec in &mut specs {
        host.context.variants.apply(spec);
        let port = query::lookup(index, &spec.name)?;
        let mut session = host.open(root, port, spec)?;
        session.set_progress(host.progress.clone());
        session.set_proxies(Some(host.context.proxies.clone()));
        session.set_mirror_worker(worker.clone());
        let progress = host.progress.clone();
        let name = spec.name.clone();
        let directory = std::env::current_dir()?;
        let phase = match target {
            "fetch" => build::Phase::Fetch,
            "checksum" => build::Phase::Checksum,
            "extract" => build::Phase::Extract,
            "patch" => build::Phase::Patch,
            "configure" => build::Phase::Configure,
            "build" => build::Phase::Build,
            _ => build::Phase::Destroot,
        };
        session.enable_system(&host.context.environment, move |request| {
            let command = request.into_command(&directory)?;
            std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()?
                            .block_on(build::run_status(&command, &name, phase, &progress))
                    })
                    .join()
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("process worker panicked")))
            })
        })?;
        for phase in phases {
            let item = session.scalar(&format!("org.macports.{phase}"))?;
            session.run_prepared_target(&item, false)?;
        }
    }
    Ok(())
}
