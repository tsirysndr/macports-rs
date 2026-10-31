//! `port install`: build each planned port from source, then install and
//! activate it through the same store, registry and activation backend that
//! binary images use.
use crate::{
    index::Index,
    install::{
        self, Action, Item,
        archive::VerifiedArchive,
        binary::Installer,
        inspect::Limits,
        source::package::{self, Inputs, Platform},
        store::Store,
    },
    portfile::Session,
    query,
    registry::write::Writer,
    spec::PortSpec,
    tcl_list,
};
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub(super) fn run(
    out: &mut impl Write,
    index: &Index,
    registry_file: Option<&Path>,
    ports: &[String],
    quiet: bool,
    root: &Path,
) -> Result<()> {
    let host = super::local::Host::new(root, registry_file, quiet)?;
    let mut specs = query::specifications(index, ports)?;
    for spec in &mut specs {
        host.context.variants.apply(spec);
    }
    let registry = host.context.open_registry(registry_file)?;
    let mut requested_specs = BTreeMap::new();
    let plan = install::plan_source_sessions(index, &specs, registry.as_ref(), |port, spec| {
        requested_specs
            .entry(port.name.clone())
            .or_insert_with(|| spec.clone());
        host.open(root, port, spec)
    })?;
    drop(registry);
    let install::SessionPlan { plan, mut sessions } = plan;
    let items: Vec<Item<'_>> = plan.items().collect();
    if items
        .iter()
        .all(|item| item.action == Action::AlreadyActive)
    {
        if !quiet {
            writeln!(out, "All requested ports are already installed and active.")?;
        }
        return Ok(());
    }

    let portdbpath = PathBuf::from(
        host.context
            .variables
            .get("portdbpath")
            .context("portdbpath is not configured")?,
    );
    let software = portdbpath.join("software");
    fs::create_dir_all(&software)
        .with_context(|| format!("cannot create {}", software.display()))?;
    let store = Store::open(&software)?;
    let registry_path = host.registry_path.clone();
    if let Some(parent) = registry_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut writer = if registry_path.exists() {
        Writer::open(&registry_path)?
    } else {
        Writer::create(&registry_path)?
    };
    let staging = tempfile::Builder::new()
        .prefix("port-rs-install.")
        .tempdir_in(&portdbpath)
        .context("cannot create installation staging directory")?;
    let machine = crate::source::detect_platform()?;
    let worker =
        crate::install::mirror_worker::MirrorWorker::new(host.context.mirror_timings.clone(), 8)?;

    for item in &items {
        match item.action {
            Action::AlreadyActive => continue,
            Action::Resolve => bail!(
                "{}: dependency providers could not be resolved: {}",
                item.port.name,
                item.provider_fallbacks.join(", ")
            ),
            Action::Activate => bail!(
                "{}: an inactive image is already installed; activating existing images is not supported yet",
                item.port.name
            ),
            Action::Install => {}
        }
        let session = sessions
            .get_mut(&item.port.name)
            .with_context(|| format!("{}: no evaluated session", item.port.name))?;
        let requested = requested_specs
            .get(&item.port.name)
            .cloned()
            .unwrap_or_else(|| PortSpec::named(&item.port.name));
        let label = super::phase::prepare(session, &host, &worker, &item.port.name)?;
        super::phase::run_targets(session, &label, &super::phase::ORDER)?;
        label.set(crate::build::Phase::Destroot);
        let portfile =
            fs::read_to_string(Path::new(&session.scalar("portpath")?).join("Portfile"))?;
        let mut installer = Installer {
            store: &store,
            registry: &mut writer,
            staging: staging.path(),
            root: Path::new("/"),
            limits: Limits::default(),
            progress: &host.progress,
        };
        installer.install_built(
            item,
            session,
            false,
            |session| {
                let reader = crate::registry::Registry::open(Some(&registry_path))?;
                install::registration::with_providers(
                    session,
                    item,
                    &machine,
                    &requested,
                    portfile.clone(),
                    reader.as_ref(),
                )
            },
            |session, target| {
                let output = staging.path().join(target.filename(item)?);
                package::create(&package_inputs(session, index)?, &output)?;
                VerifiedArchive::local(fs::File::open(&output)?)
            },
        )?;
    }
    Ok(())
}

fn optional(session: &mut Session, name: &str) -> Option<String> {
    session.scalar(name).ok().filter(|value| !value.is_empty())
}

/// Control-file inputs for `portinstall::create_archive` from the live session.
fn package_inputs(session: &mut Session, index: &Index) -> Result<Inputs> {
    let subport = session.scalar("subport")?;
    let workpath = PathBuf::from(session.scalar("workpath")?);
    let variants = session
        .active_variants()?
        .split('+')
        .filter(|variant| !variant.is_empty())
        .map(String::from)
        .collect();
    let mut dependencies = Vec::new();
    for field in ["depends_lib", "depends_run"] {
        for dependency in tcl_list::parse(&session.scalar(field).unwrap_or_default())? {
            let name = dependency.rsplit(':').next().unwrap_or(&dependency);
            if let Ok(port) = query::lookup(index, name) {
                let revision = match port.get("revision") {
                    "" => "0",
                    value => value,
                };
                dependencies.push(format!("{}-{}_{revision}", port.name, port.get("version")));
            }
        }
    }
    let platforms = session.scalar("platforms")?;
    let entries = tcl_list::parse(&platforms)?;
    let any_platform = entries.iter().any(|entry| entry == "any");
    let any_major = entries.iter().any(|entry| {
        tcl_list::parse(entry).is_ok_and(|parts| parts.get(1).is_some_and(|major| major == "any"))
    });
    let platform = (!any_platform).then(|| {
        let use_xcode = optional(session, "use_xcode")
            .is_some_and(|value| matches!(value.as_str(), "1" | "yes" | "true" | "on"));
        let xcode = optional(session, "xcodeversion").filter(|v| v != "none");
        let clt = optional(session, "xcodecltversion").filter(|v| v != "none");
        let toolchain = match (use_xcode, xcode, clt) {
            (true, Some(version), _) => Some(("xcodeversion".to_string(), version)),
            (_, _, Some(version)) => Some(("xcodecltversion".to_string(), version)),
            _ => None,
        };
        Platform {
            subplatform: optional(session, "os.subplatform").unwrap_or_default(),
            os_version: (!any_major)
                .then(|| optional(session, "os.version"))
                .flatten(),
            macos_version: optional(session, "macos_version"),
            toolchain,
        }
    });
    Ok(Inputs {
        destroot: PathBuf::from(session.scalar("destroot")?),
        statefile: workpath.join(format!(".macports.{subport}.state")),
        portfile: Path::new(&session.scalar("portpath")?).join("Portfile"),
        version: session.scalar("version")?,
        revision: session.scalar("revision")?.parse().unwrap_or(0),
        epoch: session.scalar("epoch")?.parse().unwrap_or(0),
        variants,
        archs: session.invoke("get_canonical_archs", &[])?,
        dependencies,
        description: optional(session, "description"),
        long_description: optional(session, "long_description"),
        homepage: optional(session, "homepage"),
        platform,
        ports_commit: None,
        source_date_epoch: session.scalar("source_date_epoch")?,
        macports_version: crate::portfile::MACPORTS_BASE_VERSION.into(),
        configured_cxx_stdlib: optional(session, "configure.cxx_stdlib").unwrap_or_default(),
        default_cxx_stdlib: optional(session, "cxx_stdlib").unwrap_or_default(),
        subport,
    })
}
