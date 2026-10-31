//! Resolve action prefixes against the complete upstream vocabulary, even when
//! this implementation cannot execute the resulting action yet.
use anyhow::{Result, bail};

const ACTIONS: &[&str] = &[
    "usage",
    "help",
    "echo",
    "info",
    "location",
    "notes",
    "provides",
    "log",
    "activate",
    "deactivate",
    "select",
    "sync",
    "selfupdate",
    "setrequested",
    "unsetrequested",
    "setunrequested",
    "clean",
    "upgrade",
    "rev-upgrade",
    "reclaim",
    "diagnose",
    "version",
    "platform",
    "prefix",
    "uninstall",
    "mirror",
    "installed",
    "outdated",
    "contents",
    "space",
    "dependents",
    "rdependents",
    "deps",
    "rdeps",
    "variants",
    "search",
    "list",
    "edit",
    "cat",
    "dir",
    "work",
    "cd",
    "url",
    "file",
    "logfile",
    "gohome",
    "fetch",
    "checksum",
    "extract",
    "patch",
    "configure",
    "build",
    "destroot",
    "install",
    "test",
    "lint",
    "livecheck",
    "distcheck",
    "bump",
    "load",
    "unload",
    "reload",
    "distfiles",
    "archivefetch",
    "archive",
    "unarchive",
    "dmg",
    "mdmg",
    "mpkg",
    "pkg",
    "snapshot",
    "restore",
    "migrate",
    "quit",
    "exit",
    "download", // Rust extension.
];

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
