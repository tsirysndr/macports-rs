//! Build sandboxing (portsandbox) and trace mode (porttrace) as macOS Seatbelt
//! profiles applied with sandbox-exec(1) to every command a target runs.
//!
//! The sandbox denies writes outside the work area (and network access with
//! `sandbox_network`). Trace mode additionally hides files in the prefix that
//! belong to ports outside the target's dependency closure, which is what
//! upstream's darwintrace does for reads, and fences writes the same way.
use crate::{portfile::Session, registry::Registry};
use anyhow::Result;
use std::{
    collections::{BTreeSet, HashSet},
    path::{Path, PathBuf},
};

pub(super) const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

fn value(session: &mut Session, name: &str) -> String {
    session.scalar(name).unwrap_or_default()
}

fn truthy(text: &str) -> bool {
    matches!(text, "1" | "yes" | "true" | "on")
}

/// Quote a path for a Seatbelt string literal.
fn quote(path: &str) -> String {
    format!("\"{}\"", path.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Dependency fields whose ports a target may see in trace mode.
fn traced_fields(target: &str) -> &'static [&'static str] {
    match target {
        "fetch" | "checksum" => &["depends_fetch"],
        "extract" => &["depends_fetch", "depends_extract"],
        "patch" => &["depends_fetch", "depends_extract", "depends_patch"],
        "configure" | "build" => &[
            "depends_fetch",
            "depends_extract",
            "depends_patch",
            "depends_lib",
            "depends_build",
        ],
        "test" => &[
            "depends_fetch",
            "depends_extract",
            "depends_patch",
            "depends_lib",
            "depends_build",
            "depends_run",
            "depends_test",
        ],
        "activate" | "install" => &["depends_lib", "depends_run"],
        _ => &[
            "depends_fetch",
            "depends_extract",
            "depends_patch",
            "depends_lib",
            "depends_build",
            "depends_run",
        ],
    }
}

/// Active files in the prefix owned by ports this target may not use.
fn hidden_files(session: &mut Session, target: &str, registry_path: &Path) -> Result<Vec<String>> {
    let Some(registry) = Registry::open(Some(registry_path)).ok().flatten() else {
        return Ok(Vec::new());
    };
    let entries = registry.entries()?;
    let edges = registry.dependent_edges()?;
    let mut allowed: BTreeSet<String> = BTreeSet::new();
    allowed.insert(value(session, "subport").to_ascii_lowercase());
    let mut pending: Vec<String> = Vec::new();
    for field in traced_fields(target) {
        for dependency in crate::tcl_list::parse(&value(session, field)).unwrap_or_default() {
            if let Some(name) = dependency.rsplit(':').next() {
                pending.push(name.to_ascii_lowercase());
            }
        }
    }
    // The recursive closure through registered runtime dependencies.
    while let Some(name) = pending.pop() {
        if !allowed.insert(name.clone()) {
            continue;
        }
        let ids: HashSet<i64> = entries
            .iter()
            .filter(|entry| entry.active && entry.name.eq_ignore_ascii_case(&name))
            .map(|entry| entry.id)
            .collect();
        for (dependency, id) in &edges {
            if ids.contains(id) {
                pending.push(dependency.to_ascii_lowercase());
            }
        }
    }
    let mut hidden = Vec::new();
    for entry in entries.iter().filter(|entry| entry.active) {
        if !allowed.contains(&entry.name.to_ascii_lowercase()) {
            hidden.extend(registry.files(entry)?);
        }
    }
    Ok(hidden)
}

/// The profile for one target, or `None` when the target runs unsandboxed.
pub(super) fn profile(
    session: &mut Session,
    target: &str,
    registry_path: &Path,
) -> Result<Option<String>> {
    let trace = crate::options::get().trace;
    let sandbox = truthy(&value(session, "sandbox_enable"));
    if !sandbox && !trace {
        return Ok(None);
    }
    if matches!(
        target,
        "activate" | "deactivate" | "dmg" | "mdmg" | "load" | "unload" | "reload"
    ) {
        return Ok(None);
    }
    let mut allow: Vec<String> = Vec::new();
    match target {
        "archive" | "install" | "uninstall" => {
            allow.push(format!("{}/software", value(session, "portdbpath")));
        }
        "fetch" | "mirror" | "clean" => allow.push(value(session, "distpath")),
        "pkg" => allow.push(value(session, "package.destpath")),
        _ => {}
    }
    allow.push(value(session, "workpath"));
    if truthy(&value(session, "configure.ccache")) {
        allow.push(value(session, "ccache_dir"));
    }
    if value(session, "build.type") == "xcode" {
        let home = format!("{}/home", value(session, "portdbpath"));
        allow.extend([
            format!("{home}/Library/Developer/Xcode"),
            format!("{home}/Library/Caches"),
            format!("{home}/Library/org.swift.swiftpm"),
            format!("{home}/.swiftpm"),
            value(session, "worksrcpath"),
        ]);
    }
    let mut directories: Vec<String> = Vec::new();
    for directory in allow.into_iter().filter(|directory| !directory.is_empty()) {
        if let Ok(resolved) = std::fs::canonicalize(&directory) {
            let resolved = resolved.to_string_lossy().into_owned();
            if resolved != directory {
                directories.push(resolved);
            }
        }
        directories.push(directory);
    }
    let mut profile = String::from(
        "(version 1) (allow default) (deny file-write*) \
(allow file-write-data (literal \"/dev/null\") (literal \"/dev/zero\") \
(literal \"/dev/dtracehelper\") (literal \"/dev/tty\") \
(literal \"/dev/stdin\") (literal \"/dev/stdout\") (literal \"/dev/stderr\") \
(literal \"/dev/random\") (literal \"/dev/urandom\") (regex #\"^/dev/fd/\")) \
(allow file-write* (regex #\"^(/private)?(/var)?/tmp/\" #\"^(/private)?/var/folders/\" #\"^(/private)?/var/db/mds/\")) \
(allow file-write-data (regex #\"^/dev/ttys\") (literal \"/dev/ptmx\")) \
(allow file-write-mode (regex #\"^/dev/ttys\"))",
    );
    for directory in &directories {
        profile.push_str(&format!(
            " (allow file-write* (subpath {}))",
            quote(directory)
        ));
        profile.push_str(&format!(
            " (allow file-write-setugid (subpath {}))",
            quote(directory)
        ));
    }
    if truthy(&value(session, "sandbox_network"))
        && !matches!(target, "fetch" | "mirror")
        && !truthy(&value(session, "configure.distcc"))
    {
        profile.push_str(" (deny network*)");
    }
    if value(session, "build.type") == "xcode" {
        profile.push_str(" (allow job-creation)");
    }
    if trace && !matches!(target, "clean" | "uninstall") {
        let hidden = hidden_files(session, target, registry_path)?;
        if !hidden.is_empty() {
            profile.push_str(" (deny file-read*");
            for file in &hidden {
                profile.push_str(&format!(" (literal {})", quote(file)));
            }
            profile.push(')');
        }
    }
    Ok(Some(profile))
}

/// Wrap a command so it runs under `profile`.
pub(super) fn wrap(command: &mut crate::build::Command, profile: &str) {
    let mut args = vec![
        "-p".into(),
        profile.into(),
        std::mem::replace(&mut command.program, PathBuf::from(SANDBOX_EXEC).into()),
    ];
    args.append(&mut command.args);
    command.args = args;
}
