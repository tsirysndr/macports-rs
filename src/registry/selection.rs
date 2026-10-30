use crate::spec::{PortSpec, is_modifier};
use anyhow::{Context, Result, ensure};

pub fn specifications(tokens: &[String]) -> Result<Vec<PortSpec>> {
    let mut specs: Vec<PortSpec> = Vec::new();
    for token in tokens {
        if is_modifier(token) {
            specs
                .last_mut()
                .context("port modifier must follow an installed port selector")?
                .parse_modifier(token)?;
        } else {
            ensure!(
                !matches!(token.as_str(), "and" | "or" | "not" | "!" | "(" | ")")
                    && !token
                        .split('@')
                        .next()
                        .unwrap_or(token)
                        .contains([':', '/']),
                "registry queries currently support names, globs, and installed/active/inactive/requested/unrequested selectors; logical and field expressions are not implemented"
            );
            let (name, version) = token
                .split_once('@')
                .map_or((token.as_str(), None), |(n, v)| (n, Some(v)));
            ensure!(!name.is_empty(), "missing installed port name");
            let mut spec = PortSpec::named(name);
            if let Some(version) = version {
                spec.parse_version(&format!("@{version}"))?;
            }
            specs.push(spec);
        }
    }
    Ok(specs)
}
