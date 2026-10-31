//! Evaluated registration properties, before inventory and archive metadata bind.
use super::{Item, target::Target};
use crate::{portfile::Session, registry::write::NewImage, source::Platform, spec::PortSpec};
use anyhow::{Context, Result, ensure};

/// Provider resolution and Portfile serialization belong to the caller. In
/// particular, bin/lib/path dependencies must not use guessed fallback names.
pub struct Inputs<'a> {
    pub requested: &'a PortSpec,
    pub dependencies: Vec<String>,
    pub portfile: String,
}

/// Derive runtime dependency names from the live Portfile and a current registry
/// snapshot. Call after dependency installation, with a fresh registry reader;
/// a planning snapshot cannot observe providers activated in the meantime.
pub fn with_providers(
    session: &mut Session,
    item: &Item<'_>,
    machine: &Platform,
    requested: &PortSpec,
    portfile: String,
    registry: Option<&crate::registry::Registry>,
) -> Result<(Target, NewImage)> {
    use crate::dependencies::provider::{Resolver, Search};
    let resolver = Resolver::new(registry, Search::from_session(session)?)?;
    let dependencies = resolver.runtime_dependencies(session)?;
    from_session(
        session,
        item,
        machine,
        Inputs {
            requested,
            dependencies,
            portfile,
        },
    )
}

pub fn from_session(
    session: &mut Session,
    item: &Item<'_>,
    machine: &Platform,
    inputs: Inputs<'_>,
) -> Result<(Target, NewImage)> {
    ensure!(
        item.variants_resolved && item.provider_fallbacks.is_empty(),
        "registration requires resolved variants and dependency providers"
    );
    let name = session.scalar("subport")?;
    let version = session.scalar("version")?;
    let revision: i64 = session
        .scalar("revision")?
        .parse()
        .context("invalid live revision")?;
    let epoch: i64 = session
        .scalar("epoch")?
        .parse()
        .context("invalid live epoch")?;
    ensure!(
        revision >= 0 && epoch >= 0,
        "negative live epoch or revision"
    );
    let variants = session.active_variants()?;
    let indexed_number = |field| -> Result<i64> {
        let value = item.port.get(field);
        Ok(if value.is_empty() {
            0
        } else {
            value
                .parse()
                .with_context(|| format!("invalid planned {field}"))?
        })
    };
    ensure!(
        name.eq_ignore_ascii_case(&item.port.name)
            && name.eq_ignore_ascii_case(&inputs.requested.name)
            && version == item.port.get("version")
            && revision == indexed_number("revision")?
            && epoch == indexed_number("epoch")?
            && variants == item.variants
            && session.scalar("portvariants")? == variants,
        "live Portfile identity changed from the installation plan; replan before registration"
    );
    let target = Target::from_session(session, machine)?;
    let (platform, os_major) = target
        .platform()
        .split_once('_')
        .context("invalid archive platform")?;
    let requested_variants = session.scalar("requested_variants")?;
    if !requested_variants.is_empty() {
        let mut requested = PortSpec::named(&name);
        requested.parse_modifier(&requested_variants)?;
        ensure!(
            requested.options.is_empty()
                && requested.version.is_none()
                && requested.composite_version() == requested_variants,
            "live requested variants are not canonical"
        );
    }
    // When old archives lack actual runtime metadata, upstream compares compiler
    // selection with the installation default, leaving actual runtime unknown.
    let configured_stdlib = session.scalar("configure.cxx_stdlib")?;
    let default_stdlib = session.scalar("cxx_stdlib")?;
    let record = NewImage {
        name,
        version,
        revision,
        epoch,
        variants,
        requested_variants,
        requested: item.requested,
        portfile: inputs.portfile,
        dependencies: inputs.dependencies,
        platform: platform.to_owned(),
        os_major: os_major.to_owned(),
        archs: target.architectures().join(" "),
        cxx_stdlib_overridden: configured_stdlib != default_stdlib,
        ..NewImage::default()
    };
    Ok((target, record))
}
