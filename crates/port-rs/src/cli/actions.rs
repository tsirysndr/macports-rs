//! Resolve action prefixes against the complete upstream vocabulary, even when
//! this implementation cannot execute the resulting action yet.
use anyhow::{Result, bail};

use port_rs::commands::ACTIONS;

pub(super) fn expand(action: &str) -> Result<String> {
    if ACTIONS.contains(&action) {
        return Ok(action.into());
    }
    let prefix = action.to_lowercase();
    let matcher = port_rs::query::Matcher::new(&format!("{prefix}*"), false, false, true)?;
    let matches: Vec<_> = ACTIONS
        .iter()
        .copied()
        .filter(|name| !matches!(*name, "cd" | "exit" | "quit"))
        .filter(|name| matcher.matches(name))
        .collect();
    match matches.as_slice() {
        [name] => Ok((*name).into()),
        [] => Ok(action.into()), // Preserve the unsupported-action diagnostic.
        _ => bail!(
            "\"port {action}\" is ambiguous:\n  port {}",
            matches.join("\n  port ")
        ),
    }
}
