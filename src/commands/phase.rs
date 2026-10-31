use crate::{build, index::Index, portfile::Session, query};
use anyhow::{Result, bail};
use clap::Args;
use std::{cell::Cell, path::Path, rc::Rc};

#[derive(Args)]
pub struct Phase {
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}

pub(super) const ORDER: [&str; 8] = [
    "main",
    "fetch",
    "checksum",
    "extract",
    "patch",
    "configure",
    "build",
    "destroot",
];

/// The phase label shown on a session's process output while it runs.
pub(super) type Label = Rc<Cell<build::Phase>>;

fn label_for(target: &str) -> build::Phase {
    match target {
        "fetch" => build::Phase::Fetch,
        "checksum" => build::Phase::Checksum,
        "extract" => build::Phase::Extract,
        "patch" => build::Phase::Patch,
        "configure" => build::Phase::Configure,
        "build" | "main" => build::Phase::Build,
        "test" => build::Phase::Test,
        _ => build::Phase::Destroot,
    }
}

/// Bind progress, proxies, the mirror worker and real process execution.
pub(super) fn prepare(
    session: &mut Session,
    host: &super::local::Host,
    worker: &crate::install::mirror_worker::MirrorWorker,
    name: &str,
) -> Result<Label> {
    session.set_progress(host.progress.clone());
    let logs = std::path::PathBuf::from(session.scalar("portdbpath")?)
        .join("logs")
        .join(session.scalar("portpath")?.replace('/', "_"))
        .join(session.scalar("subport")?);
    if let Err(err) = host.progress.open_log(&logs.join("main.log")) {
        host.progress
            .warning(&format!("cannot open log {}: {err}", logs.display()));
    }
    session.set_proxies(Some(host.context.proxies.clone()));
    session.set_mirror_worker(worker.clone());
    let progress = host.progress.clone();
    let name = name.to_string();
    let directory = std::env::current_dir()?;
    let label: Label = Rc::new(Cell::new(build::Phase::Build));
    let current = label.clone();
    session.enable_system(&host.context.environment, move |request| {
        let command = request.into_command(&directory)?;
        let phase = current.get();
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
    Ok(label)
}

/// Run the given targets in order, labelling process output with each one.
pub(super) fn run_targets(session: &mut Session, label: &Label, targets: &[&str]) -> Result<()> {
    for target in targets {
        label.set(label_for(target));
        if let Some(progress) = session.progress() {
            progress.set_log_phase(target);
        }
        let item = session.scalar(&format!("org.macports.{target}"))?;
        session.run_prepared_target(&item, false)?;
    }
    Ok(())
}

/// Run the requested source phase and its prerequisites for each port in order.
pub(super) fn run(
    index: &Index,
    registry_file: Option<&Path>,
    args: Phase,
    target: &str,
    quiet: bool,
    root: &Path,
) -> Result<()> {
    let targets: Vec<&str> = if target == "test" {
        ORDER.iter().copied().chain(["test"]).collect()
    } else {
        let Some(end) = ORDER.iter().position(|phase| *phase == target) else {
            bail!("unsupported phase {target}");
        };
        ORDER[..=end].to_vec()
    };
    let mut specs = query::specifications(index, &args.ports)?;
    let host = super::local::Host::new(root, registry_file, quiet)?;
    let worker =
        crate::install::mirror_worker::MirrorWorker::new(host.context.mirror_timings.clone(), 8)?;
    for spec in &mut specs {
        host.context.variants.apply(spec);
        let port = query::lookup(index, &spec.name)?;
        let mut session = host.open(root, port, spec)?;
        let label = prepare(&mut session, &host, &worker, &spec.name)?;
        run_targets(&mut session, &label, &targets)?;
    }
    Ok(())
}
