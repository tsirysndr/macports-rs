//! Native Portfile metadata evaluation. No tclsh, libtcl, or subprocess fallback.
//! This is an incremental MacPorts command implementation, not yet a complete
//! PortSystem. Unsupported commands fail; deferred build hooks are retained.
use crate::accounts;
mod activate_runtime;
mod architecture;
mod archive_defaults;
mod archive_files;
mod archive_paths;
mod archive_runtime;
mod archive_state;
pub use archive_defaults::ArchiveSites;
mod binaries;
mod build_defaults;
mod build_phase;
mod build_settings;
mod build_state;
mod command_exec;
mod command_options;
mod commands;
mod configure_checks;
mod configure_phase;
mod configure_start;
mod creation_mask;
mod defaults;
mod destroot_directories;
mod destroot_layout;
mod destroot_manpages;
pub use destroot_directories::DestrootDirectories;
pub use destroot_manpages::ManpageCompression;
mod destroot_libtool;
pub use destroot_layout::DestrootLayout;
mod destroot_finish;
mod destroot_mtree;
mod destroot_phase;
mod destroot_start;
mod stage_context;
mod startupitem;
pub use destroot_libtool::LibtoolCleanup;
use stage_context::StageContext;
pub use utilities::MACPORTS_BASE_VERSION;
mod builtin_callbacks;
mod channels;
mod checksum_runtime;
mod compilers;
mod diagnostics;
mod ditems;
mod dlist;
mod exec;
mod extract_phase;
mod fetch_dns;
mod fetch_files;
mod fetch_pings;
mod fetch_requests;
mod fetch_resources;
mod fetch_runtime;
mod fetch_selection;
mod fetch_sites;
mod fetch_start;
mod file_copy;
mod file_delete;
mod file_link;
mod file_normalize;
mod file_ownership;
mod file_permissions;
mod file_rename;
mod file_times;
mod filesystem;
mod filesystem_mutations;
mod groups;
mod install_start;
mod mirror_times;
mod option_procs;
mod patch_phase;
mod paths;
mod phase_start;
mod platform;
mod registry_lookup;
mod session;
mod source_epoch;
mod state_open;
mod user_commands;
pub use session::Session;
pub use variants::Declaration;
mod source;
pub use source::{
    evaluate_from_tree, open_from_tree, open_from_tree_with_archive_sites,
    open_from_tree_with_environment, open_from_tree_with_registry,
};
mod system;
pub use system::SystemRequest;
mod target_execution;
mod target_runtime;
mod targets;
pub use target_execution::TargetOutcome;
mod test_phase;
mod traverse;
mod utilities;
mod workpath;
pub use groups::{LoadedGroup, PortGroups};
mod variants;
use crate::{index::Port, spec::PortSpec};
use anyhow::{Result, ensure};
use molt::{ContextID, Exception, Interp, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    pub description: String,
    pub requires: Vec<String>,
    pub conflicts: Vec<String>,
    pub body: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hook {
    pub phase: String,
    pub body: String,
}
#[derive(Clone, PartialEq, Eq)]
pub struct Evaluation {
    pub groups: Vec<LoadedGroup>,
    pub port: Port,
    pub variants: BTreeMap<String, Variant>,
    pub enabled: String,
    pub hooks: Vec<Hook>,
    /// Custom phase bodies, replacing the built-in implementation for that phase.
    /// These are deferred scripts, not executed or validated build actions.
    pub targets: BTreeMap<String, String>,
    /// Opaque dependency-item handles and their final metadata. These records
    /// do not imply that the associated phase procedures can execute yet.
    pub dependency_items: BTreeMap<String, BTreeMap<String, String>>,
}
#[derive(Default)]
struct State {
    archive_sites: Option<ArchiveSites>,
    registry_lookup: Option<registry_lookup::Source>,
    groups: PortGroups,
    mirror_times: crate::install::mirror_times::MirrorTimings,
    mirror_worker: Option<crate::install::mirror_worker::MirrorWorker>,
    mirror_resources: BTreeMap<String, crate::install::sites::resource::Resource>,
    progress: Option<crate::progress::Progress>,
    proxies: Option<std::sync::Arc<crate::install::proxy::Resolved>>,
    dmg_mount: Option<String>,
    /// Lazy defaults that need execution inputs; metadata snapshots skip them
    /// until a Portfile assigns a value.
    execution_defaults: BTreeSet<String>,
    option_procs: BTreeMap<String, Vec<String>>,
    variable_traces: BTreeMap<String, Vec<(Vec<String>, String)>>,
    tracing: BTreeSet<String>,
    callbacks: Vec<String>,
    variant_descriptions: Option<BTreeMap<String, String>>,
    source_date_epoch: Option<i64>,
    channels: BTreeMap<String, channels::Channel>,
    next_channel: u64,
    warned: BTreeSet<String>,
    notifications: Vec<(String, String)>,
    build_states: BTreeMap<String, crate::build::state::StateFile>,
    next_build_state: u64,
    creation_mask: Option<u32>,
    system_executor: Option<system::Executor>,
    loaded_groups: Vec<LoadedGroup>,
    group_failures: BTreeSet<String>,
    options: BTreeSet<String>,
    overrides: BTreeSet<String>,
    variants: BTreeMap<String, Variant>,
    order: Vec<String>,
    /// Variants whose Portfile declaration carried its own description.
    described_variants: BTreeSet<String>,
    selected: BTreeMap<String, bool>,
    requested: BTreeMap<String, bool>,
    defaults: BTreeMap<String, bool>,
    lazy_defaults: BTreeMap<String, String>,
    resolving_defaults: BTreeSet<String>,
    changing_format: bool,
    hooks: Vec<Hook>,
    targets: BTreeMap<String, String>,
    ditems: ditems::Items,
    selected_subport: bool,
    active: BTreeSet<String>,
    unsupported: BTreeSet<String>,
}
/// Configuration values upstream copies into each port's `system_options`.
const SYSTEM_OPTIONS: &[&str] = &[
    "portdbpath",
    "porturl",
    "portpath",
    "portbuildpath",
    "auto_path",
    "prefix",
    "prefix_frozen",
    "portsharepath",
    "registry.path",
    "registry.format",
    "user_home",
    "user_path",
    "user_ssh_auth_sock",
    "portarchivetype",
    "portarchive_hfscompression",
    "archivefetch_pubkeys",
    "portautoclean",
    "portimage_mode",
    "porttrace",
    "keeplogs",
    "portverbose",
    "destroot_umask",
    "rsync_server",
    "rsync_options",
    "rsync_dir",
    "startupitem_autostart",
    "startupitem_type",
    "startupitem_install",
    "place_worksymlink",
    "macportsuser",
    "sudo_user",
    "configureccache",
    "ccache_dir",
    "ccache_size",
    "configuredistcc",
    "configurepipe",
    "buildnicevalue",
    "buildmakejobs",
    "applications_dir",
    "applications_dir_frozen",
    "current_phase",
    "frameworks_dir",
    "frameworks_dir_frozen",
    "developer_dir",
    "universal_archs",
    "build_arch",
    "os_arch",
    "os_endian",
    "os_version",
    "os_major",
    "os_minor",
    "os_platform",
    "os_subplatform",
    "macos_version",
    "macos_version_major",
    "macosx_version",
    "macosx_sdk_version",
    "macosx_deployment_target",
    "packagemaker_path",
    "default_compilers",
    "sandbox_enable",
    "sandbox_network",
    "delete_la_files",
    "cxx_stdlib",
    "pkg_post_unarchive_deletions",
    "clonebin_path",
    "macosx_sdk_path",
];

fn error(message: impl Into<String>) -> Exception {
    Exception::molt_err2("PORTFILE".into(), Value::from(message.into()))
}
fn failure(error: Exception) -> anyhow::Error {
    anyhow::anyhow!("Portfile evaluation: {}", error.value())
}
fn global(interp: &mut Interp, name: &str) {
    if interp.scope_level() > 0 {
        interp.upvar(0, name);
    }
}
fn list(values: impl IntoIterator<Item = String>) -> Value {
    Value::from(values.into_iter().map(Value::from).collect::<Vec<_>>())
}

/// Evaluate source supplied by the caller, using explicit platform/configuration
/// variables. Does not search for PortGroups or execute hooks. Supported read-only
/// filesystem queries inspect the host when invoked by the source.
/// Callers must not treat success as approval to skip returned hooks or targets.
pub fn evaluate(
    source: &str,
    variables: &BTreeMap<String, String>,
    requested: &PortSpec,
) -> Result<Evaluation> {
    evaluate_with_groups(source, variables, requested, PortGroups::default())
}

/// Evaluate with caller-selected local or embedded PortGroup sources. Imported
/// source is fingerprinted for later registry recording; fingerprints do not
/// establish trust. Failed imports remain fatal even if script code catches them.
pub fn evaluate_with_groups(
    source: &str,
    variables: &BTreeMap<String, String>,
    requested: &PortSpec,
    groups: PortGroups,
) -> Result<Evaluation> {
    Ok(Session::open(source, variables, requested, groups)?.into_metadata())
}

fn open(
    source: &str,
    variables: &BTreeMap<String, String>,
    requested: &PortSpec,
    groups: PortGroups,
    environment: Option<&BTreeMap<String, String>>,
    archive_sites: Option<ArchiveSites>,
    registry_lookup: Option<registry_lookup::Source>,
) -> Result<Session> {
    let mut interp = Interp::new();
    interp.set_recursion_limit(128);
    interp.eval("unset env").map_err(failure)?;
    if let Some(environment) = environment {
        let values = environment
            .iter()
            .flat_map(|(name, value)| [Value::from(name.clone()), Value::from(value.clone())])
            .collect::<Vec<_>>();
        interp.array_set("env", &values).map_err(failure)?;
    }
    let mirror_times = crate::install::mirror_times::MirrorTimings::from_variables(variables)?;
    let id = interp.save_context(State {
        mirror_times,
        archive_sites,
        registry_lookup,
        creation_mask: crate::source::startup_creation_mask(),
        groups,
        selected: requested.variants.clone(),
        requested: requested.variants.clone(),
        ..State::default()
    });
    interp.array_set("variations", &[]).map_err(failure)?;
    for (name, enabled) in &requested.variants {
        variants::mark(&mut interp, name, *enabled, false).map_err(failure)?;
    }
    for command in [
        "source", "exit", "parse", "pdump", "pclear", "socket", "load", "default", "platform",
        "system",
    ] {
        interp.add_context_command(command, commands::unsupported, id);
    }
    for name in commands::OPTIONS {
        commands::register(&mut interp, id, name);
    }
    command_options::install(&mut interp, id).map_err(failure)?;
    command_exec::install(&mut interp, id);
    for (name, value) in [
        ("name", requested.name.as_str()),
        ("subport", requested.name.as_str()),
        ("revision", "0"),
        ("epoch", "0"),
        ("platforms", "darwin"),
    ] {
        interp.set_scalar(name, value.into()).map_err(failure)?;
    }
    for (name, value) in variables {
        interp
            .set_scalar(name, value.clone().into())
            .map_err(failure)?;
        if SYSTEM_OPTIONS.contains(&name.as_str()) {
            interp
                .set_element("system_options", name, value.clone().into())
                .map_err(failure)?;
        }
    }
    for (name, value) in &requested.options {
        interp
            .set_scalar(name, value.clone().into())
            .map_err(failure)?;
        interp.context::<State>(id).overrides.insert(name.clone());
    }
    if let Ok(defaults) = interp.scalar("default_variants") {
        variants::defaults(&mut interp, id, &defaults.as_list().map_err(failure)?)
            .map_err(failure)?;
    }
    commands::install(&mut interp, id);
    utilities::install(&mut interp, id);
    workpath::install(&mut interp, id);
    build_state::install(&mut interp, id);
    channels::install(&mut interp, id);
    archive_state::install(&mut interp, id);
    archive_paths::install(&mut interp, id);
    archive_runtime::install(&mut interp, id);
    fetch_sites::install(&mut interp, id);
    fetch_files::install(&mut interp, id).map_err(failure)?;
    fetch_resources::install(&mut interp, id);
    fetch_runtime::install(&mut interp, id);
    fetch_requests::install(&mut interp, id);
    diagnostics::install(&mut interp, id);
    user_commands::install(&mut interp, id);
    exec::install(&mut interp, id);
    checksum_runtime::install(&mut interp, id);
    extract_phase::install(&mut interp, id);
    patch_phase::install(&mut interp, id);
    fetch_pings::install(&mut interp, id);
    fetch_dns::install(&mut interp, id);
    fetch_selection::install(&mut interp, id);
    fetch_start::install(&mut interp, id);
    mirror_times::install(&mut interp, id);
    registry_lookup::install(&mut interp, id);
    phase_start::install(&mut interp, id);
    install_start::install(&mut interp, id);
    activate_runtime::install(&mut interp, id);
    destroot_finish::install(&mut interp, id);
    destroot_start::install(&mut interp, id);
    startupitem::install(&mut interp, id);
    configure_checks::install(&mut interp, id);
    configure_start::install(&mut interp, id);
    paths::install(&mut interp, id);
    creation_mask::install(&mut interp, id);
    traverse::install(&mut interp, id);
    ditems::install(&mut interp, id);
    dlist::install(&mut interp, id);
    targets::install(&mut interp, id).map_err(failure)?;
    interp.add_context_command("PortGroup", groups::command, id);
    interp.add_context_command("eval_variants", variants::eval_command, id);
    interp.add_context_command("platform", platform::command, id);
    defaults::install(&mut interp, id).map_err(failure)?;
    archive_defaults::install(&mut interp, id).map_err(failure)?;
    build_settings::install(&mut interp, id).map_err(failure)?;
    compilers::install(&mut interp, id).map_err(failure)?;
    build_defaults::register_paths(&mut interp, id).map_err(failure)?;
    build_defaults::register_execution(&mut interp, id).map_err(failure)?;
    startupitem::initialize(&mut interp, id).map_err(failure)?;
    source_epoch::install(&mut interp, id).map_err(failure)?;
    option_procs::install(&mut interp, id).map_err(failure)?;
    builtin_callbacks::install(&mut interp, id);
    build_phase::install(&mut interp, id);
    test_phase::install(&mut interp, id);
    configure_phase::install(&mut interp, id);
    destroot_phase::install(&mut interp, id);
    architecture::install(&mut interp, id);
    archive_files::install(&mut interp, id);
    build_defaults::install(&mut interp, id);
    binaries::install(&mut interp, id);
    interp.eval(source).map_err(failure)?;
    for (name, _) in &requested.options {
        ensure!(
            interp.context::<State>(id).options.contains(name),
            "unknown port option {name}"
        );
    }
    interp.eval("universal_setup").map_err(failure)?;
    interp.eval("eval_variants variations").map_err(failure)?;
    interp.eval("port::run_callbacks").map_err(failure)?;
    let options = interp.context::<State>(id).options.clone();
    let mut fields = BTreeMap::new();
    for name in options {
        let state = interp.context::<State>(id);
        if state.execution_defaults.contains(&name) && state.lazy_defaults.contains_key(&name) {
            continue;
        }
        if interp.var_exists(&Value::from(name.clone())) {
            let lazy = interp
                .context::<State>(id)
                .lazy_defaults
                .contains_key(&name);
            let unsupported = interp.context::<State>(id).unsupported.clone();
            match interp.scalar(&name) {
                Ok(_) if lazy && interp.context::<State>(id).unsupported != unsupported => {
                    interp.context::<State>(id).unsupported = unsupported;
                }
                Ok(value) => {
                    fields.insert(name, value.as_str().to_owned());
                }
                // Upstream metadata never computes unexported defaults; one that
                // cannot be computed yet is omitted rather than fatal.
                Err(_) if lazy => interp.context::<State>(id).unsupported = unsupported,
                Err(err) => return Err(failure(err)),
            }
        }
    }
    check_supported(&mut interp, id)?;
    ensure!(
        fields.get("version").is_some_and(|v| !v.is_empty()),
        "Portfile did not set version"
    );
    let state = interp.context::<State>(id);
    ensure!(
        fields
            .get("name")
            .is_some_and(|name| name == &requested.name)
            || state.selected_subport,
        "Portfile does not define requested port {}",
        requested.name
    );
    if let Some(version) = &requested.version {
        let revision = fields.get("revision").map(String::as_str).unwrap_or("0");
        ensure!(
            version == &fields["version"]
                || version == &format!("{}_{revision}", fields["version"]),
            "Portfile version does not match requested version {version}"
        );
    }
    fields.insert("name".into(), requested.name.clone());
    fields.insert("variants".into(), list(state.order.clone()).to_string());
    let mut metadata = Vec::new();
    for (name, variant) in &state.variants {
        let mut details = Vec::new();
        if !variant.description.is_empty() {
            details.extend(["description".into(), variant.description.clone()]);
        }
        for (key, values) in [
            ("requires", &variant.requires),
            ("conflicts", &variant.conflicts),
        ] {
            if !values.is_empty() {
                details.extend([key.into(), list(values.clone()).to_string()]);
            }
        }
        if let Some(selected) = state.defaults.get(name) {
            details.extend([
                "is_default".into(),
                if *selected { "+" } else { "-" }.into(),
            ]);
        }
        metadata.extend([name.clone(), list(details).to_string()]);
    }
    fields.insert("vinfo".into(), list(metadata).to_string());
    let enabled: String = state.active.iter().map(|name| format!("+{name}")).collect();
    fields.insert("canonical_active_variants".into(), enabled.clone());
    let metadata = Evaluation {
        groups: state.loaded_groups.clone(),
        port: Port {
            name: requested.name.clone(),
            fields,
        },
        enabled,
        variants: state.variants.clone(),
        hooks: state.hooks.clone(),
        targets: state.targets.clone(),
        dependency_items: state.ditems.snapshot(),
    };
    Ok(Session {
        interp,
        id,
        metadata,
    })
}

fn check_supported(interp: &mut Interp, id: ContextID) -> Result<()> {
    ensure!(
        interp.context::<State>(id).unsupported.is_empty(),
        "Portfile used unsupported commands: {}",
        interp
            .context::<State>(id)
            .unsupported
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    );
    ensure!(
        interp.context::<State>(id).group_failures.is_empty(),
        "Portfile imports did not complete: {}",
        interp
            .context::<State>(id)
            .group_failures
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("; ")
    );
    Ok(())
}
