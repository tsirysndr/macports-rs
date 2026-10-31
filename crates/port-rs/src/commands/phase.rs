use crate::{build, index::Index, portfile::Session, query};
use anyhow::{Result, bail};
use clap::Args;
use std::{cell::Cell, path::Path, rc::Rc};

#[derive(Args)]
pub struct Phase {
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}

#[derive(Args)]
pub struct Fetch {
    /// Fetch only from the port's own sites, not the MacPorts mirrors
    #[arg(long = "no-mirrors")]
    pub no_mirrors: bool,
    #[command(flatten)]
    pub phase: Phase,
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

/// The running target: the phase shown on process output and the sandbox
/// profile its commands run under.
pub(super) struct Running {
    phase: Cell<build::Phase>,
    profile: std::cell::RefCell<Option<String>>,
    registry: std::path::PathBuf,
    trace: std::cell::RefCell<Option<super::trace::Tracer>>,
    hidden: std::cell::RefCell<std::collections::HashSet<String>>,
    progress: crate::progress::Progress,
}
impl Running {
    /// Select `target`'s phase label, sandbox profile and trace log.
    pub(super) fn enter(&self, session: &mut Session, target: &str) -> Result<()> {
        self.phase.set(label_for(target));
        let traced = super::sandbox::traced(target);
        let hidden = if traced {
            super::sandbox::hidden_files(session, target, &self.registry)?
        } else {
            Vec::new()
        };
        let tracer = if traced {
            let mut value = |name: &str| session.scalar(name).unwrap_or_default();
            let portdbpath = std::path::PathBuf::from(value("portdbpath"));
            let ignored = vec![
                value("portdbpath"),
                value("workpath"),
                value("distpath"),
                value("portpath"),
            ];
            super::trace::Tracer::new(
                &portdbpath,
                &value("prefix"),
                std::path::Path::new(&value("workpath")),
                target,
                ignored,
            )?
        } else {
            None
        };
        let sipdir = tracer.as_ref().map(|tracer| tracer.sipdir.clone());
        *self.profile.borrow_mut() =
            super::sandbox::profile(session, target, &hidden, sipdir.as_deref())?;
        *self.hidden.borrow_mut() = hidden.into_iter().collect();
        *self.trace.borrow_mut() = tracer;
        Ok(())
    }

    /// Report what trace mode saw during the target just run.
    pub(super) fn finish(&self) {
        if let Some(tracer) = self.trace.borrow_mut().take() {
            tracer.report(&self.hidden.borrow(), &self.registry, &self.progress);
        }
    }
}
pub(super) type Label = Rc<Running>;

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
    let label: Label = Rc::new(Running {
        phase: Cell::new(build::Phase::Build),
        profile: Default::default(),
        registry: host.registry_path.clone(),
        trace: Default::default(),
        hidden: Default::default(),
        progress: host.progress.clone(),
    });
    let current = label.clone();
    session.enable_system(&host.context.environment, move |request| {
        let mut command = request.into_command(&directory)?;
        if let Some(tracer) = current.trace.borrow().as_ref() {
            tracer.wrap(&mut command)?;
        }
        if let Some(profile) = current.profile.borrow().as_deref() {
            super::sandbox::wrap(&mut command, profile);
        }
        let phase = current.phase.get();
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
        label.enter(session, target)?;
        if let Some(progress) = session.progress() {
            progress.set_log_phase(target);
        }
        let item = session.scalar(&format!("org.macports.{target}"))?;
        let result = session.run_prepared_target(&item, false);
        label.finish();
        if let Err(err) = result {
            let message = err.to_string();
            let message = message
                .strip_prefix("Portfile evaluation: ")
                .unwrap_or(&message);
            let failure = format!(
                "Failed to {target} {}: {message}",
                session.scalar("subport")?
            );
            let Some(progress) = session.progress() else {
                return Err(anyhow::anyhow!(failure));
            };
            progress.log("error", &failure);
            match progress.log_path() {
                Some(log) => bail!("{failure}\nError: See {} for details.", log.display()),
                None => bail!("{failure}"),
            }
        }
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
