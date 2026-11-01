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
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

/// How requested ports are installed. Without `activate`, they are only built
/// and stored (`port archive`, `install --no-activate`); their dependencies
/// are still installed and activated.
#[derive(Clone, Copy)]
pub(super) struct Mode {
    pub activate: bool,
    pub requested: bool,
    /// Build requested ports again even if the same image is installed.
    pub rebuild: bool,
}

impl Mode {
    pub const INSTALL: Self = Self {
        activate: true,
        requested: true,
        rebuild: false,
    };
    pub const ARCHIVE: Self = Self {
        activate: false,
        requested: true,
        rebuild: false,
    };
}

pub(super) fn run(
    out: &mut impl Write,
    index: &Index,
    registry_file: Option<&Path>,
    args: &super::install::Install,
    quiet: bool,
    root: &Path,
) -> Result<()> {
    let mut specs = query::specifications(index, &args.ports)?;
    let progress = crate::progress::Progress::new(quiet);
    for spec in &mut specs {
        let port = query::lookup(index, &spec.name)?;
        let replacement = port.get("replaced_by");
        if !replacement.is_empty() && !args.no_replace {
            progress.message(&format!("{} is replaced by {replacement}", port.name));
            spec.name = query::lookup(index, replacement)
                .with_context(|| format!("Port {replacement} not found"))?
                .name
                .clone();
        }
        let port = query::lookup(index, &spec.name)?;
        if matches!(port.get("known_fail"), "yes" | "1" | "true" | "on") && !args.allow_failing {
            ensure!(
                super::ask(
                    &format!("{} is known to fail. Try to install anyway?", port.name),
                    false
                ),
                "{} is known to fail (use --allow-failing to try to install anyway)",
                port.name
            );
        }
    }
    let mode = Mode {
        activate: !args.no_activate,
        requested: !args.unrequested,
        rebuild: false,
    };
    run_specs(out, index, registry_file, specs, quiet, root, mode)
}

/// Install already parsed specifications (with explicit variants).
pub(super) fn run_specs(
    out: &mut impl Write,
    index: &Index,
    registry_file: Option<&Path>,
    mut specs: Vec<PortSpec>,
    quiet: bool,
    root: &Path,
    mode: Mode,
) -> Result<()> {
    let activate = mode.activate;
    let host = super::local::Host::new(root, registry_file, quiet)?;
    for spec in &mut specs {
        host.context.variants.apply(spec);
    }
    let registry = host.context.open_registry(registry_file)?;
    let mut requested_specs = BTreeMap::new();
    let machine = crate::source::detect_platform()?;
    let plan = install::plan_install_sessions(index, &specs, registry.as_ref(), |port, spec| {
        let first = !requested_specs.contains_key(&port.name);
        requested_specs
            .entry(port.name.clone())
            .or_insert_with(|| spec.clone());
        if first
            && specs
                .iter()
                .any(|requested| requested.name.eq_ignore_ascii_case(&port.name))
        {
            host.progress
                .phase(&format!("Computing dependencies for {}", port.name));
        }
        let mut session = host.open(root, port, spec)?;
        let binary = super::archivefetch::available(&host, &mut session, &machine);
        Ok((session, binary))
    })?;
    drop(registry);
    let install::SessionPlan { plan, mut sessions } = plan;
    let items: Vec<Item<'_>> = plan.items().collect();
    if items
        .iter()
        .all(|item| item.action == Action::AlreadyActive && !(mode.rebuild && item.requested))
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
    // A MacPorts prefix always exists once base is installed; create it so a
    // fresh standalone prefix is writable for install and activation.
    if let Some(prefix) = host.context.variables.get("prefix") {
        fs::create_dir_all(prefix).with_context(|| format!("cannot create prefix {prefix}"))?;
    }
    let software = portdbpath.join("software");
    fs::create_dir_all(&software)
        .with_context(|| format!("cannot create {}", software.display()))?;
    let store = Store::open(&software)?;
    let registry_path = host.registry_path.clone();
    if let Some(parent) = registry_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !registry_path.exists() {
        drop(Writer::create(&registry_path)?);
    }
    let staging = tempfile::Builder::new()
        .prefix("port-rs-install.")
        .tempdir_in(&portdbpath)
        .context("cannot create installation staging directory")?;
    let worker =
        crate::install::mirror_worker::MirrorWorker::new(host.context.mirror_timings.clone(), 8)?;

    let mut notes = Vec::new();
    let options = crate::options::get();
    let autoclean = options.autoclean.unwrap_or_else(|| {
        host.context
            .variables
            .get("portautoclean")
            .is_none_or(|value| matches!(value.as_str(), "yes" | "1" | "true" | "on"))
    });
    let source_preference = host
        .context
        .variables
        .get("buildfromsource")
        .cloned()
        .unwrap_or_default();
    let archive_mode = match (super::archive_mode(), source_preference.as_str()) {
        (super::ArchiveMode::Any, "always") => super::ArchiveMode::SourceOnly,
        (super::ArchiveMode::Any, "never") => super::ArchiveMode::BinaryOnly,
        (mode, _) => mode,
    };
    // Download every archive the plan needs at once, before installing in order.
    let mut prefetched = if archive_mode == super::ArchiveMode::SourceOnly {
        Default::default()
    } else {
        let wanted: Vec<&Item<'_>> = items
            .iter()
            .filter(|item| !options.nodeps || item.requested)
            .filter(|item| {
                item.action == Action::Install
                    || (mode.rebuild && item.requested && item.action != Action::Resolve)
            })
            .collect();
        super::archivefetch::prefetch(&host, &mut sessions, &wanted, &machine, staging.path())
    };
    let mut ready = super::archivefetch::prepare_all(
        &host,
        &mut sessions,
        &items,
        &mut prefetched,
        &machine,
        staging.path(),
    );
    for item in &items {
        // -n: act on the requested ports only, not their dependencies.
        if options.nodeps && !item.requested {
            continue;
        }
        let rebuild = mode.rebuild && item.requested;
        match item.action {
            Action::AlreadyActive | Action::Activate if rebuild => {}
            Action::AlreadyActive => continue,
            Action::Resolve => bail!(
                "{}: dependency providers could not be resolved: {}",
                item.port.name,
                item.provider_fallbacks.join(", ")
            ),
            Action::Activate if !activate && item.requested => {
                host.progress.phase(&format!(
                    "{} @{}{} is already archived",
                    item.port.name,
                    item.port.version(),
                    item.variants
                ));
                continue;
            }
            Action::Activate => {
                let entry = installed_image(&registry_path, item)?;
                super::lifecycle::activate_entry(
                    &registry_path,
                    &entry,
                    &host.progress,
                    staging.path(),
                )?;
                if item.requested && mode.requested && !entry.requested {
                    let mut writer = Writer::open(&registry_path)?;
                    let mut transaction = writer.transaction()?;
                    transaction.set_requested(entry.id, true)?;
                    transaction.commit()?;
                }
                continue;
            }
            Action::Install => {}
        }
        let activate = activate || !item.requested;
        let adjusted;
        let item = if (item.requested && !mode.requested) || rebuild {
            adjusted = Item {
                port: item.port,
                variants: item.variants.clone(),
                variants_resolved: item.variants_resolved,
                provider_fallbacks: item.provider_fallbacks.clone(),
                requested: item.requested && mode.requested,
                action: Action::Install,
            };
            &adjusted
        } else {
            item
        };
        if !sessions.contains_key(&item.port.name) {
            let spec = requested_specs
                .get(&item.port.name)
                .cloned()
                .unwrap_or_else(|| PortSpec::named(&item.port.name));
            sessions.insert(item.port.name.clone(), host.open(root, item.port, &spec)?);
        }
        let session = sessions
            .get_mut(&item.port.name)
            .with_context(|| format!("{}: no evaluated session", item.port.name))?;
        let requested = requested_specs
            .get(&item.port.name)
            .cloned()
            .unwrap_or_else(|| PortSpec::named(&item.port.name));
        let keeplogs = host
            .context
            .variables
            .get("keeplogs")
            .is_some_and(|value| matches!(value.as_str(), "yes" | "1" | "true" | "on"));
        let label = super::phase::prepare(session, &host, &worker, &item.port.name)?;
        // As upstream, a compatible signed archive replaces building from source.
        let mut binary = prefetched.remove(&item.port.name);
        if binary.is_none() && archive_mode != super::ArchiveMode::SourceOnly {
            match super::archivefetch::fetch_archive(&host, session, item, &machine, staging.path())
            {
                Ok(archive) => binary = Some(archive),
                Err(err) if archive_mode == super::ArchiveMode::BinaryOnly => {
                    bail!(
                        "{}: binary-only mode, but no archive is available: {err:#}",
                        item.port.name
                    )
                }
                Err(err) => host.progress.log(
                    "debug",
                    &format!("no binary archive for {}: {err:#}", item.port.name),
                ),
            }
        }
        if binary.is_none() {
            super::phase::run_targets(session, &label, &super::phase::ORDER)?;
        }
        label.enter(session, "install")?;
        host.progress.set_log_phase("install");
        let portfile = Path::new(&session.scalar("portpath")?).join("Portfile");
        // A forced rebuild replaces the identical image it was built from.
        if rebuild && let Some(existing) = identical_image(&registry_path, item)? {
            super::lifecycle::uninstall_entry(&registry_path, &existing, &host.progress)?;
        }
        // Hold the registry write lock only while this port installs.
        let mut writer = Writer::open(&registry_path)?;
        // As upstream activation does, retire another active version first.
        if activate && let Some(reader) = crate::registry::Registry::open(Some(&registry_path))? {
            let active = reader
                .entries()?
                .into_iter()
                .find(|entry| entry.active && entry.name.eq_ignore_ascii_case(&item.port.name));
            if let Some(active) = active {
                let files = reader.files(&active)?;
                drop(reader);
                host.progress.phase(&format!(
                    "Deactivating {} @{}",
                    active.name,
                    active.full_version()
                ));
                install::deactivation::deactivate(&mut writer, active.id, &files)?;
            }
        }
        let mut installer = Installer {
            store: &store,
            registry: &mut writer,
            staging: staging.path(),
            root: Path::new("/"),
            limits: Limits::default(),
            progress: &host.progress,
            prepared: ready.remove(&item.port.name),
        };
        installer.install_built(
            item,
            session,
            false,
            activate,
            |session| {
                let reader = crate::registry::Registry::open(Some(&registry_path))?;
                let (target, mut record) = install::registration::with_providers(
                    session,
                    item,
                    &machine,
                    &requested,
                    String::new(),
                    reader.as_ref(),
                )?;
                save_portfile(&registry_path, session, &portfile, &mut record)?;
                Ok((target, record))
            },
            |session, target| {
                if let Some(archive) = binary {
                    return Ok(archive);
                }
                let output = staging.path().join(target.filename(item)?);
                package::create(&package_inputs(session, index)?, &output)?;
                VerifiedArchive::local(fs::File::open(&output)?)
            },
        )?;
        notes.extend(session.take_notifications());
        // Successful installs drop their log unless keeplogs is set.
        host.progress.close_log();
        if !keeplogs {
            let logs = Path::new(&session.scalar("portdbpath")?)
                .join("logs")
                .join(session.scalar("portpath")?.replace('/', "_"));
            let _ = fs::remove_dir_all(logs.join(session.scalar("subport")?));
            let _ = fs::remove_dir(&logs);
        }
        if autoclean {
            host.progress
                .phase(&format!("Cleaning {}", session.scalar("subport")?));
            let _ = fs::remove_dir_all(session.scalar("subbuildpath")?);
        }
    }
    if !notes.is_empty() && !quiet {
        writeln!(out, "--->  Some of the ports you installed have notes:")?;
        for (port, text) in notes {
            writeln!(out, "  {port} has the following notes:")?;
            for note in tcl_list::parse(&text).unwrap_or_else(|_| vec![text.clone()]) {
                for line in note.trim_matches('\n').lines() {
                    writeln!(out, "    {}", line.trim_start())?;
                }
            }
        }
    }
    Ok(())
}

/// Keep the Portfile and its PortGroups in the registry, as upstream does, so
/// later activate, deactivate and uninstall can run the port's hooks.
fn save_portfile(
    registry_path: &Path,
    session: &mut Session,
    portfile: &Path,
    record: &mut crate::registry::write::NewImage,
) -> Result<()> {
    use crate::registry::saved;
    let registry = saved::registry_directory(registry_path);
    record.portfile = saved::save_portfile(
        registry,
        &record.name,
        &record.version,
        record.revision,
        portfile,
    )?;
    record.portgroups = session
        .loaded_groups()
        .into_iter()
        .filter_map(|group| Some((group.path?, group.name, group.version)))
        .map(|(path, name, version)| saved::save_group(registry, &name, &version, &path))
        .collect::<Result<_>>()?;
    Ok(())
}

/// A registered image, active or not, with the planned item's identity.
fn identical_image(
    registry_path: &Path,
    item: &Item<'_>,
) -> Result<Option<crate::registry::Entry>> {
    let revision = item.port.get("revision").parse::<i64>().unwrap_or(0);
    let Some(registry) = crate::registry::Registry::open(Some(registry_path))? else {
        return Ok(None);
    };
    Ok(registry.entries()?.into_iter().find(|entry| {
        entry.name.eq_ignore_ascii_case(&item.port.name)
            && entry.version == item.port.get("version")
            && entry.revision == revision
            && entry.variants == item.variants
    }))
}

/// The registered, inactive image matching a planned item.
fn installed_image(registry_path: &Path, item: &Item<'_>) -> Result<crate::registry::Entry> {
    let revision = item.port.get("revision").parse::<i64>().unwrap_or(0);
    let epoch = item.port.get("epoch");
    crate::registry::Registry::open(Some(registry_path))?
        .context("registry is missing")?
        .entries()?
        .into_iter()
        .find(|entry| {
            !entry.active
                && entry.name.eq_ignore_ascii_case(&item.port.name)
                && entry.version == item.port.get("version")
                && entry.revision == revision
                && entry.variants == item.variants
                && entry.epoch.parse::<i64>().unwrap_or(0) == epoch.parse::<i64>().unwrap_or(0)
        })
        .with_context(|| format!("{}: installed image not found", item.port.name))
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
