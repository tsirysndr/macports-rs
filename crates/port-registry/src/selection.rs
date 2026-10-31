use crate::spec::{PortSpec, is_modifier};
use anyhow::{Context, Result, ensure};

/// Installed-port specifications. Pseudo-portnames stay names here: the
/// registry expression evaluator gives them their meaning.
pub fn specifications(tokens: &[String]) -> Result<Vec<PortSpec>> {
    parse(tokens, false)
}

/// Like `specifications`, with pseudo-portnames (`inactive`, `leaves`,
/// `current`, ...) expanded, for actions that take a plain list of images.
pub fn expanded_specifications(tokens: &[String]) -> Result<Vec<PortSpec>> {
    parse(tokens, true)
}

fn parse(tokens: &[String], expand: bool) -> Result<Vec<PortSpec>> {
    let mut specs: Vec<PortSpec> = Vec::new();
    for token in tokens {
        if is_modifier(token) {
            specs
                .last_mut()
                .context("port modifier must follow an installed port selector")?
                .parse_modifier(token)?;
        } else if expand
            && crate::query::is_pseudo(token)
            && let Some(expanded) = crate::query::resolve_pseudo(token, true)?
        {
            specs.extend(expanded);
        } else {
            ensure!(
                !matches!(token.as_str(), "and" | "or" | "not" | "!" | "(" | ")")
                    && !token
                        .split('@')
                        .next()
                        .unwrap_or(token)
                        .contains([':', '/']),
                "this selector is not implemented here; field/path selectors require catalog queries and logical operators require an expression-capable command"
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
