//! Livecheck setup (portlivecheck): resolve `livecheck.type` and load the
//! ports tree's type defaults from `_resources/port1.0/livecheck`.
use super::*;
use anyhow::Result;
use molt::{MoltResult, check_args};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("getdefaultportresourcepath", resource_path, id);
    interp.add_context_command("port_rs::livecheck_types", types, id);
    interp.add_context_command("port_rs::source_resource", source_resource, id);
}

fn resources(interp: &mut Interp, id: ContextID) -> Result<std::path::PathBuf, Exception> {
    interp
        .context::<State>(id)
        .groups
        .resource_directory
        .clone()
        .ok_or_else(|| error("no ports tree resources are available"))
}

fn resource_path(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 2, "?path?")?;
    let root = resources(interp, id)?;
    let path = match argv.get(1) {
        Some(path) => root.join(path.as_str()),
        None => root,
    };
    Ok(path.to_string_lossy().into_owned().into())
}

fn types(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    let directory = resources(interp, id)?.join("port1.0/livecheck");
    let mut names: Vec<String> = std::fs::read_dir(&directory)
        .map_err(|_| {
            error(format!(
                "No available types were found. Check '{}'.",
                directory.display()
            ))
        })?
        .flatten()
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .strip_suffix(".tcl")
                .map(str::to_string)
        })
        .collect();
    names.sort();
    Ok(Value::from(
        names.into_iter().map(Value::from).collect::<Vec<_>>(),
    ))
}

/// Evaluate a script from the ports tree's `_resources` at global scope, with
/// `source` available to it for other resources only.
fn source_resource(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "path")?;
    let root = std::fs::canonicalize(resources(interp, id)?)
        .map_err(|err| error(format!("cannot resolve resources: {err}")))?;
    let path = std::fs::canonicalize(argv[1].as_str())
        .map_err(|err| error(format!("couldn't read file \"{}\": {err}", argv[1])))?;
    if !path.starts_with(&root) {
        return commands::unsupported(interp, id, &[Value::from("source")]);
    }
    let script = std::fs::read_to_string(&path)
        .map_err(|err| error(format!("couldn't read file \"{}\": {err}", path.display())))?;
    interp.add_context_command("source", source_resource, id);
    let result = interp.with_global_scope(|interp| interp.eval(&script));
    interp.add_context_command("source", commands::unsupported, id);
    result
}

const PLAN: &str = r#"
set has_master_sites [info exists master_sites]
set has_homepage [info exists homepage]
if {!$has_homepage} {
    set livecheck.url {}
}
set available_types [join [port_rs::livecheck_types] |]
if {${livecheck.type} eq "default"} {
    if {$has_master_sites} {
        foreach master_site ${master_sites} {
            if {[regexp "^($available_types)(?::(\[^:\]+))?" $master_site _ site subdir]} {
                set subdirs [split $subdir /]
                if {[llength $subdirs] > 1} {
                    if {[lindex $subdirs 0] eq "project"} {
                        set subdir [lindex $subdirs 1]
                    } else {
                        set subdir ""
                    }
                }
                if {$subdir ne "" && ${livecheck.name} eq "default"} {
                    set livecheck.name $subdir
                }
                set livecheck.type $site
                break
            }
        }
    }
    if {${livecheck.type} eq "default"} {
        set livecheck.type fallback
    }
    if {$has_homepage} {
        if {[regexp {^http://code.google.com/p/([^/]+)} $homepage _ tag]} {
            set livecheck.type googlecode
        } elseif {[regexp {^http://www.gnu.org/software/([^/]+)} $homepage _ tag]} {
            set livecheck.type gnu
        }
    }
}
if {${livecheck.type} in [split $available_types |]} {
    port_rs::source_resource [getdefaultportresourcepath port1.0/livecheck/${livecheck.type}.tcl]
}
set livecheck.url [join ${livecheck.url}]
list ${livecheck.type} ${livecheck.url} [join ${livecheck.regex}] ${livecheck.md5} \
    ${livecheck.branch} ${livecheck.version} ${livecheck.ignore_sslcert} ${subport}
"#;

/// The resolved livecheck configuration of a port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivecheckPlan {
    pub kind: String,
    pub url: String,
    pub regex: String,
    pub md5: String,
    pub branch: String,
    pub version: String,
    pub ignore_sslcert: bool,
    pub subport: String,
}

impl Session {
    pub fn livecheck_plan(&mut self) -> Result<LivecheckPlan> {
        let result = self.invoke("uplevel", &["#0", PLAN])?;
        let fields = crate::tcl_list::parse(&result)?;
        let field = |index: usize| fields.get(index).cloned().unwrap_or_default();
        Ok(LivecheckPlan {
            kind: field(0),
            url: field(1),
            regex: field(2),
            md5: field(3),
            branch: field(4),
            version: field(5),
            ignore_sslcert: matches!(field(6).as_str(), "yes" | "1" | "true" | "on"),
            subport: field(7),
        })
    }
}
