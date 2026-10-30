use crate::{index::Port, spec::PortSpec, tcl_list};
use anyhow::{Result, ensure};
use std::collections::BTreeSet;

pub(super) fn defaults(port: &Port, requested: Option<&PortSpec>) -> Result<String> {
    let available: BTreeSet<_> = tcl_list::parse(port.get("variants"))?.into_iter().collect();
    let metadata = tcl_list::dictionary(port.get("vinfo"))?;
    let mut enabled = BTreeSet::new();
    for name in &available {
        ensure!(
            metadata.contains_key(name) || port.fields.contains_key("default_variants"),
            "default variant metadata for {name} is missing; Portfile evaluation is required"
        );
        if let Some(raw) = metadata.get(name) {
            let detail = tcl_list::dictionary(raw)?;
            if detail.get("is_default").is_some_and(|value| value == "+") {
                enabled.insert(name.clone());
            }
        }
    }
    // Some index producers record default_variants directly instead of vinfo.
    for value in tcl_list::parse(port.get("default_variants"))? {
        let mut spec = PortSpec::named(&port.name);
        spec.parse_modifier(&value)?;
        for (name, selected) in spec.variants {
            ensure!(
                available.contains(&name),
                "unknown indexed default variant {name}"
            );
            if selected {
                enabled.insert(name);
            } else {
                enabled.remove(&name);
            }
        }
    }
    if let Some(spec) = requested {
        ensure!(
            spec.options.is_empty(),
            "port options require Portfile evaluation, which is not implemented"
        );
        for (name, selected) in &spec.variants {
            ensure!(available.contains(name), "unknown variant {name}");
            ensure!(
                *selected == enabled.contains(name),
                "non-default variant {name} requires Portfile evaluation; indexed installation plans support default variants only"
            );
        }
    }
    Ok(enabled.into_iter().map(|name| format!("+{name}")).collect())
}
