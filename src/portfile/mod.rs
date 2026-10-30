//! Native Portfile metadata evaluation. No tclsh, libtcl, or subprocess fallback.
//! This is an incremental MacPorts command implementation, not yet a complete
//! PortSystem. Unsupported commands fail; deferred build hooks are retained.
mod architecture;
mod binaries;
mod build_defaults;
mod build_phase;
mod build_settings;
mod command_exec;
mod command_options;
mod commands;
mod configure_phase;
mod creation_mask;
mod defaults;
mod destroot_phase;
mod ditems;
mod dlist;
mod file_delete;
mod file_link;
mod file_normalize;
mod file_permissions;
mod file_rename;
mod file_times;
mod filesystem;
mod filesystem_mutations;
mod groups;
mod paths;
mod platform;
mod session;
pub use session::Session;
mod source;
pub use source::{evaluate_from_tree, open_from_tree};
mod system;
pub use system::SystemRequest;
mod targets;
mod test_phase;
mod traverse;
mod utilities;
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
#[derive(PartialEq, Eq)]
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
    groups: PortGroups,
    creation_mask: Option<u32>,
    system_executor: Option<system::Executor>,
    loaded_groups: Vec<LoadedGroup>,
    group_failures: BTreeSet<String>,
    options: BTreeSet<String>,
    overrides: BTreeSet<String>,
    variants: BTreeMap<String, Variant>,
    order: Vec<String>,
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
) -> Result<Session> {
    let mut interp = Interp::new();
    interp.set_recursion_limit(128);
    interp.eval("unset env").map_err(failure)?;
    let id = interp.save_context(State {
        creation_mask: crate::source::startup_creation_mask(),
        groups,
        selected: requested.variants.clone(),
        requested: requested.variants.clone(),
        ..State::default()
    });
    for command in [
        "source", "exit", "puts", "parse", "pdump", "pclear", "exec", "open", "socket", "load",
        "default", "platform", "system",
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
    paths::install(&mut interp, id);
    creation_mask::install(&mut interp, id);
    traverse::install(&mut interp, id);
    ditems::install(&mut interp, id);
    dlist::install(&mut interp, id);
    targets::install(&mut interp, id).map_err(failure)?;
    interp.add_context_command("PortGroup", groups::command, id);
    interp.add_context_command("platform", platform::command, id);
    defaults::install(&mut interp, id).map_err(failure)?;
    build_settings::install(&mut interp, id).map_err(failure)?;
    build_phase::install(&mut interp, id);
    test_phase::install(&mut interp, id);
    configure_phase::install(&mut interp, id);
    destroot_phase::install(&mut interp, id);
    architecture::install(&mut interp, id);
    build_defaults::install(&mut interp, id);
    binaries::install(&mut interp, id);
    interp.eval(source).map_err(failure)?;
    for (name, _) in &requested.options {
        ensure!(
            interp.context::<State>(id).options.contains(name),
            "unknown port option {name}"
        );
    }
    variants::apply(&mut interp, id).map_err(failure)?;
    let options = interp.context::<State>(id).options.clone();
    let mut fields = BTreeMap::new();
    for name in options {
        if interp.var_exists(&Value::from(name.clone())) {
            let value = interp.scalar(&name).map_err(failure)?;
            fields.insert(name, value.as_str().to_owned());
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
