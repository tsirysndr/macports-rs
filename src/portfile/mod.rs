//! Native Portfile metadata evaluation. No tclsh, libtcl, or subprocess fallback.
//! This is an incremental MacPorts command implementation, not yet a complete
//! PortSystem. Unsupported commands fail; deferred build hooks are retained.
mod commands;
mod variants;
use crate::{index::Port, spec::PortSpec};
use anyhow::{Result, ensure};
use molt::{ContextID, Exception, Interp, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct Variant {
    pub description: String,
    pub requires: Vec<String>,
    pub conflicts: Vec<String>,
    pub body: String,
}
#[derive(Debug)]
pub struct Hook {
    pub phase: String,
    pub body: String,
}
pub struct Evaluation {
    pub port: Port,
    pub variants: BTreeMap<String, Variant>,
    pub enabled: String,
    pub hooks: Vec<Hook>,
}
#[derive(Default)]
struct State {
    options: BTreeSet<String>,
    overrides: BTreeSet<String>,
    variants: BTreeMap<String, Variant>,
    order: Vec<String>,
    selected: BTreeMap<String, bool>,
    requested: BTreeMap<String, bool>,
    defaults: BTreeMap<String, bool>,
    hooks: Vec<Hook>,
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
/// variables. Does not load PortGroups, touch the filesystem, or execute hooks.
/// Callers must not treat success as approval to skip the returned hooks.
pub fn evaluate(
    source: &str,
    variables: &BTreeMap<String, String>,
    requested: &PortSpec,
) -> Result<Evaluation> {
    let mut interp = Interp::new();
    interp.set_recursion_limit(128);
    interp.eval("unset env").map_err(failure)?;
    let id = interp.save_context(State {
        selected: requested.variants.clone(),
        requested: requested.variants.clone(),
        ..State::default()
    });
    for command in [
        "source",
        "exit",
        "puts",
        "parse",
        "pdump",
        "pclear",
        "exec",
        "open",
        "socket",
        "load",
        "PortGroup",
        "default",
        "platform",
    ] {
        interp.add_context_command(command, commands::unsupported, id);
    }
    for name in commands::OPTIONS {
        commands::register(&mut interp, id, name);
    }
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
        ensure!(
            interp.context::<State>(id).options.contains(name),
            "unknown port option {name}"
        );
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
    interp.eval(source).map_err(failure)?;
    variants::apply(&mut interp, id).map_err(failure)?;
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
    let options = interp.context::<State>(id).options.clone();
    let mut fields = BTreeMap::new();
    for name in options {
        if let Ok(value) = interp.scalar(&name) {
            fields.insert(name, value.as_str().to_owned());
        }
    }
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
    let enabled = state.active.iter().map(|name| format!("+{name}")).collect();
    Ok(Evaluation {
        port: Port {
            name: requested.name.clone(),
            fields,
        },
        enabled,
        variants: std::mem::take(&mut state.variants),
        hooks: std::mem::take(&mut state.hooks),
    })
}
