//! Option traces (`option_proc`), post-evaluation callbacks, and the default
//! universal variant, as PortSystem runs them after sourcing a Portfile.
use super::*;
use molt::{MoltResult, check_args, interp::ScalarChange};

pub(super) fn install(interp: &mut Interp, id: ContextID) -> MoltResult {
    interp.add_context_command("option_proc", option_proc, id);
    interp.add_context_command("port::register_callback", register_callback, id);
    interp.add_context_command("port::run_callbacks", run_callbacks, id);
    interp.add_context_command("universal_setup", universal_setup, id);
    interp.add_context_command("variant_delete", variant_delete, id);
    interp.add_context_command("options_export", options_export, id);
    interp.add_context_command("options::export", export, id);
    interp.add_context_command("trace", trace, id);
    interp.context::<State>(id).option_procs.extend(
        EXPORTED
            .iter()
            .map(|name| ((*name).to_string(), vec!["options::export".to_string()])),
    );
    for name in [
        "universal_variant",
        "use_configure",
        "os.universal_supported",
    ]
    .into_iter()
    .chain(UPSTREAM_OPTIONS)
    {
        commands::register(interp, id, name);
    }
    interp.set_scalar(
        "portchecksum::checksum_types",
        "md5 sha1 rmd160 sha256 size".into(),
    )?;
    for (name, value) in UPSTREAM_DEFAULTS {
        if !interp.var_exists(&(*name).into()) {
            defaults::register(interp, id, name, value)?;
            interp
                .context::<State>(id)
                .execution_defaults
                .insert((*name).into());
        }
    }
    let supported = if interp.var_exists(&"os.subplatform".into()) {
        "[expr {${os.subplatform} eq \"macosx\"}]"
    } else {
        "0"
    };
    for (name, value) in [
        ("use_configure", "yes"),
        ("universal_variant", "${use_configure}"),
        ("os.universal_supported", supported),
    ] {
        defaults::register(interp, id, name, value)?;
    }
    Ok(Value::empty())
}

/// PortSystem options declared across port1.0 that other modules do not own.
const UPSTREAM_OPTIONS: [&str; 56] = [
    "activate.asroot",
    "build.type.add_deps",
    "bzr.revision",
    "bzr.url",
    "copy_log_files",
    "cvs.date",
    "cvs.method",
    "cvs.module",
    "cvs.password",
    "cvs.root",
    "cvs.tag",
    "deactivate.asroot",
    "depends",
    "depends_skip_archcheck",
    "distcheck.type",
    "distpath",
    "filesdir",
    "git.branch",
    "git.cmd",
    "git.url",
    "hg.cmd",
    "hg.tag",
    "hg.url",
    "install.asroot",
    "install.group",
    "install.user",
    "installs_libs",
    "known_fail",
    "libpath",
    "license_noconflict",
    "load.asroot",
    "os.arch",
    "os.endian",
    "os.major",
    "os.minor",
    "os.platform",
    "os.subplatform",
    "os.version",
    "portdbpath",
    "portsandbox_active",
    "portsandbox_profile",
    "portsandbox_supported",
    "prefix",
    "provides",
    "reload.asroot",
    "sources_conf",
    "svn.method",
    "svn.revision",
    "svn.url",
    "uninstall.asroot",
    "universal_possible",
    "unload.asroot",
    "use_xcode",
    "worksrcdir",
    "worksrcpath",
    "destpath",
];

const UPSTREAM_DEFAULTS: &[(&str, &str)] = &[
    ("activate.asroot", "no"),
    ("build.type.add_deps", "yes"),
    ("bzr.revision", "-1"),
    ("cvs.method", "export"),
    ("cvs.module", "${distname}"),
    ("distcheck.type", "moddate"),
    (
        "git.cmd",
        "[findBinary git ${portutil::autoconf::git_path}]",
    ),
    ("portutil::autoconf::git_path", "/usr/bin/git"),
    ("hg.cmd", "[findBinary hg ${portutil::autoconf::hg_path}]"),
    ("portutil::autoconf::hg_path", ""),
    ("hg.tag", "tip"),
    ("install.asroot", "no"),
    ("install.user", "${portutil::autoconf::install_user}"),
    ("install.group", "${portutil::autoconf::install_group}"),
    ("portutil::autoconf::install_user", "root"),
    ("portutil::autoconf::install_group", "admin"),
    ("svn.method", "export"),
    ("uninstall.asroot", "no"),
    (
        "universal_possible",
        "[expr {${os.universal_supported} && [llength ${configure.universal_archs}] >= 2}]",
    ),
    ("portsandbox_supported", "0"),
    ("portsandbox_active", "0"),
    ("portsandbox_profile", ""),
];

/// Options mirrored into the PortInfo array, as upstream `options_export` does.
const EXPORTED: [&str; 27] = [
    "name",
    "version",
    "revision",
    "epoch",
    "categories",
    "maintainers",
    "platforms",
    "description",
    "long_description",
    "notes",
    "homepage",
    "license",
    "provides",
    "conflicts",
    "replaced_by",
    "installs_libs",
    "license_noconflict",
    "patchfiles",
    "known_fail",
    "depends_fetch",
    "depends_extract",
    "depends_patch",
    "depends_build",
    "depends_lib",
    "depends_run",
    "depends_test",
    "subport",
];

fn options_export(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    let state = interp.context::<State>(id);
    for option in &argv[1..] {
        state
            .option_procs
            .entry(option.to_string())
            .or_default()
            .push("options::export".into());
    }
    Ok(Value::empty())
}

fn export(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 4, "option action ?value?")?;
    let option = argv[1].as_str();
    interp.with_global_scope(|interp| {
        match argv[2].as_str() {
            "set" => {
                let value = argv.get(3).cloned().unwrap_or_else(Value::empty);
                interp.set_element("PortInfo", option, value)?;
            }
            "delete" => interp.unset_element("PortInfo", option),
            _ => {}
        }
        Ok(Value::empty())
    })
}

fn option_proc(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 0, "option ?proc ...?")?;
    let option = argv[1].as_str().trim_start_matches("::").to_string();
    interp
        .context::<State>(id)
        .option_procs
        .entry(option)
        .or_default()
        .extend(argv[2..].iter().map(Value::to_string));
    Ok(Value::empty())
}

fn handlers(interp: &mut Interp, id: ContextID, name: &str) -> Option<Vec<String>> {
    let state = interp.context::<State>(id);
    if state.option_procs.is_empty() || state.tracing.contains(name) {
        return None;
    }
    state.option_procs.get(name).cloned()
}

/// `trace add|remove|info variable name ?ops cmd?` for global variables.
fn trace(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 4, 6, "option type name ?ops command?")?;
    if argv[2].as_str() != "variable" {
        return Err(error(format!("trace {} is not supported", argv[2])));
    }
    let name = argv[3].as_str().trim_start_matches("::").to_string();
    let state = interp.context::<State>(id);
    match (argv[1].as_str(), argv.len()) {
        ("add", 6) => {
            let ops = argv[4].as_list()?.iter().map(Value::to_string).collect();
            state
                .variable_traces
                .entry(name)
                .or_default()
                .push((ops, argv[5].to_string()));
            Ok(Value::empty())
        }
        ("remove", 6) => {
            let ops: Vec<String> = argv[4].as_list()?.iter().map(Value::to_string).collect();
            if let Some(traces) = state.variable_traces.get_mut(&name)
                && let Some(position) = traces
                    .iter()
                    .position(|(o, command)| *o == ops && command == argv[5].as_str())
            {
                traces.remove(position);
            }
            Ok(Value::empty())
        }
        ("info", 4) => Ok(Value::from(
            state
                .variable_traces
                .get(&name)
                .map(|traces| {
                    traces
                        .iter()
                        .map(|(ops, command)| {
                            Value::from(vec![
                                Value::from(
                                    ops.iter()
                                        .map(|o| Value::from(o.as_str()))
                                        .collect::<Vec<_>>(),
                                ),
                                Value::from(command.as_str()),
                            ])
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        )),
        _ => Err(error(
            "wrong # args: should be \"trace add|remove|info variable name ?ops command?\"",
        )),
    }
}

fn variable_traces(interp: &mut Interp, id: ContextID, name: &str, op: &str) -> Vec<String> {
    let state = interp.context::<State>(id);
    if state.variable_traces.is_empty() || state.tracing.contains(name) {
        return Vec::new();
    }
    state
        .variable_traces
        .get(name)
        .map(|traces| {
            traces
                .iter()
                .filter(|(ops, _)| ops.iter().any(|o| o == op))
                .map(|(_, command)| command.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn fire_traces(interp: &mut Interp, id: ContextID, name: &str, op: &str) -> Result<(), Exception> {
    let commands = variable_traces(interp, id, name, op);
    if commands.is_empty() {
        return Ok(());
    }
    interp.context::<State>(id).tracing.insert(name.into());
    let result = commands.into_iter().try_for_each(|command| {
        let mut words = Value::from(command).as_list()?.to_vec();
        words.extend([Value::from(name), Value::empty(), Value::from(op)]);
        call(interp, words).map(|_| ())
    });
    interp.context::<State>(id).tracing.remove(name);
    if op == "unset" { Ok(()) } else { result }
}

fn call(interp: &mut Interp, words: Vec<Value>) -> MoltResult {
    interp.with_global_scope(|interp| interp.eval_value(&Value::from(words)))
}

/// Run read traces before a global option is read. As with Tcl traces, the
/// option's own traces are suspended while its handlers run.
pub(super) fn on_read(interp: &mut Interp, id: ContextID, name: &str) -> Result<(), Exception> {
    fire_traces(interp, id, name, "read")?;
    let Some(procs) = handlers(interp, id, name) else {
        return Ok(());
    };
    interp.context::<State>(id).tracing.insert(name.into());
    let result = procs.into_iter().try_for_each(|handler| {
        call(interp, vec![handler.into(), name.into(), "read".into()]).map(|_| ())
    });
    interp.context::<State>(id).tracing.remove(name);
    result
}

pub(super) fn on_change(
    interp: &mut Interp,
    id: ContextID,
    name: &str,
    change: ScalarChange,
) -> Result<(), Exception> {
    let op = if change == ScalarChange::Write {
        "write"
    } else {
        "unset"
    };
    fire_traces(interp, id, name, op)?;
    let Some(procs) = handlers(interp, id, name) else {
        return Ok(());
    };
    interp.context::<State>(id).tracing.insert(name.into());
    let result = (|| {
        if change == ScalarChange::Write {
            let value = interp.with_global_scope(|interp| interp.scalar(name))?;
            for handler in procs {
                call(
                    interp,
                    vec![handler.into(), name.into(), "set".into(), value.clone()],
                )?;
            }
        } else {
            for handler in procs {
                // Upstream logs and ignores failures from delete handlers.
                let _ = call(interp, vec![handler.into(), name.into(), "delete".into()]);
            }
        }
        Ok(())
    })();
    interp.context::<State>(id).tracing.remove(name);
    result
}

fn register_callback(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "callback")?;
    interp
        .context::<State>(id)
        .callbacks
        .push(argv[1].to_string());
    Ok(Value::empty())
}

fn run_callbacks(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    let callbacks = std::mem::take(&mut interp.context::<State>(id).callbacks);
    // Upstream calls each callback from the run_callbacks procedure; keep that
    // frame so callbacks' relative uplevels resolve identically.
    let frame = Value::from(vec![
        Value::from(vec![Value::from("callback")]),
        Value::from("$callback"),
    ]);
    for callback in callbacks {
        call(interp, vec!["apply".into(), frame.clone(), callback.into()])?;
    }
    Ok(Value::empty())
}

fn variant_delete(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "name")?;
    let name = argv[1].as_str();
    let state = interp.context::<State>(id);
    state.variants.remove(name);
    state.order.retain(|variant| variant != name);
    Ok(Value::empty())
}

fn enabled(interp: &mut Interp, name: &str) -> Result<bool, Exception> {
    Ok(interp.var_exists(&name.into()) && interp.scalar(name)?.as_bool().unwrap_or(false))
}

fn universal_setup(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 0, "?args?")?;
    interp.with_global_scope(|interp| {
        let archs = interp.scalar("configure.universal_archs")?.as_list()?.len();
        if interp
            .context::<State>(id)
            .variants
            .contains_key("universal")
        {
            if archs < 2 {
                call(interp, vec!["variant_delete".into(), "universal".into()])?;
            }
            return Ok(Value::empty());
        }
        let declined = interp.var_exists(&"universal_variant".into())
            && !interp.scalar("universal_variant")?.as_bool()?;
        if declined
            || enabled(interp, "use_xmkmf")?
            || !enabled(interp, "os.universal_supported")?
            || archs <= 1
        {
            return Ok(Value::empty());
        }
        let compiler = interp.scalar("configure.compiler")?;
        let supported = call(
            interp,
            vec![
                "portconfigure::arch_flag_supported".into(),
                compiler,
                "yes".into(),
            ],
        )?;
        if supported.as_bool()? {
            call(
                interp,
                vec!["variant".into(), "universal".into(), Value::empty()],
            )?;
        }
        Ok(Value::empty())
    })
}
