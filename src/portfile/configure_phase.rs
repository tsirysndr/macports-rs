//! Configure-stage orchestration with explicitly initialized compiler inputs.
use super::*;
use molt::MoltResult;
const ENV_OPTIONS: &[&str] = &[
    "CC",
    "CXX",
    "OBJC",
    "OBJCXX",
    "FC",
    "F77",
    "F90",
    "JAVAC",
    "CFLAGS",
    "CPPFLAGS",
    "CXXFLAGS",
    "OBJCFLAGS",
    "OBJCXXFLAGS",
    "FFLAGS",
    "F90FLAGS",
    "FCFLAGS",
    "LDFLAGS",
    "LIBS",
    "CLASSPATH",
    "PERL",
    "PYTHON",
    "RUBY",
    "INSTALL",
    "AWK",
    "BISON",
    "PKG_CONFIG",
    "DEVELOPER_DIR",
];
pub(super) fn install(interp: &mut Interp, id: ContextID) {
    for name in [
        "use_autoreconf",
        "use_automake",
        "use_autoconf",
        "use_xmkmf",
        "compiler.limit_flags",
    ] {
        commands::register(interp, id, name);
    }
    interp.add_context_command("portconfigure::configure_main", main, id);
}
fn call(interp: &mut Interp, args: Vec<Value>) -> MoltResult {
    interp.eval_value(&Value::from(args))
}
fn enabled(interp: &mut Interp, name: &str) -> Result<bool, Exception> {
    Ok(interp.var_exists(&name.into()) && interp.scalar(name)?.as_bool()?)
}
fn append(interp: &mut Interp, key: &str, values: impl IntoIterator<Item = Value>) -> MoltResult {
    let mut args = vec![
        "append_to_environment_value".into(),
        "configure".into(),
        key.into(),
    ];
    args.extend(values);
    call(interp, args)
}
fn execute(interp: &mut Interp, command: &str) -> MoltResult {
    call(interp, vec!["command_exec".into(), command.into()])
        .map_err(|err| error(format!("{command} failure: {}", err.value())))
}
fn main(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        for command in ["autoreconf", "automake", "autoconf"] {
            if enabled(interp, &format!("use_{command}"))? {
                execute(interp, command)?;
            }
        }
        if enabled(interp, "use_xmkmf")? {
            call(interp, vec!["parse_environment".into(), "xmkmf".into()])?;
            execute(interp, "xmkmf")?;
            call(interp, vec!["parse_environment".into(), "xmkmf".into()])?;
            let source = interp.scalar("worksrcpath")?;
            call(
                interp,
                vec![
                    "command_exec".into(),
                    "-varprefix".into(),
                    "xmkmf".into(),
                    format!("cd {source} && make Makefiles").into(),
                ],
            )
            .map_err(|err| error(format!("make Makefiles failure: {}", err.value())))?;
        } else if enabled(interp, "use_configure")? {
            prepare(interp)?;
            if let Err(err) = execute(interp, "configure") {
                let configure = interp.scalar("configure.dir")?;
                let build = interp.scalar("build.dir")?;
                for path in [
                    format!("{configure}/config.log"),
                    format!("{configure}/CMakeFiles/CMakeError.log"),
                    format!("{build}/meson-logs/meson-log.txt"),
                ] {
                    if filesystem::predicate("exists", &path) {
                        eprintln!(
                            "Error: Failed to configure {}: consult {path}",
                            interp.scalar("subport")?
                        );
                    }
                }
                return Err(err);
            }
        }
        Ok(0.into())
    })
}
fn prepare(interp: &mut Interp) -> MoltResult {
    call(interp, vec!["parse_environment".into(), "configure".into()])?;
    let ccache = enabled(interp, "configure.ccache")?;
    let distcc = enabled(interp, "configure.distcc")?;
    if ccache && distcc {
        append(interp, "CCACHE_PREFIX", ["distcc".into()])?;
    }
    let filter = if ccache {
        "ccache"
    } else if distcc {
        "distcc"
    } else {
        ""
    };
    for key in ["CC", "CXX", "OBJC", "OBJCXX"] {
        append(interp, key, [filter.into()])?;
    }
    let pipe = if enabled(interp, "configure.pipe")? {
        "-pipe"
    } else {
        ""
    };
    for key in [
        "CFLAGS",
        "CXXFLAGS",
        "OBJCFLAGS",
        "OBJCXXFLAGS",
        "FFLAGS",
        "F90FLAGS",
        "FCFLAGS",
    ] {
        append(interp, key, [pipe.into()])?;
    }
    for key in ENV_OPTIONS {
        let value = interp.scalar(&format!("configure.{}", key.to_ascii_lowercase()))?;
        append(interp, key, value.as_list()?.iter().cloned())?;
    }
    let path = interp.scalar("configure.pkg_config_path")?;
    let path = path
        .as_list()?
        .iter()
        .map(Value::as_str)
        .collect::<Vec<_>>()
        .join(":");
    append(interp, "PKG_CONFIG_PATH", [path.into()])?;
    if interp.scalar("os.platform")?.as_str() == "darwin"
        && interp.scalar("os.major")?.as_int()? == 12
    {
        append(interp, "__CFPREFERENCES_AVOID_DAEMON", ["1".into()])?;
    }
    let sdk = interp.scalar("configure.sdkroot")?;
    if !sdk.as_str().is_empty() && !interp.scalar("compiler.limit_flags")?.as_bool()? {
        for key in ["CPPFLAGS", "CFLAGS", "CXXFLAGS", "OBJCFLAGS", "OBJCXXFLAGS"] {
            append(interp, key, [format!("-isysroot{sdk}").into()])?;
        }
        append(interp, "LDFLAGS", [format!("-Wl,-syslibroot,{sdk}").into()])?;
    }
    for (tool, key) in [
        ("cc", "CFLAGS"),
        ("f77", "FFLAGS"),
        ("cxx", "CXXFLAGS"),
        ("objc", "OBJCFLAGS"),
        ("objcxx", "OBJCXXFLAGS"),
        ("cpp", "CPPFLAGS"),
        ("f90", "F90FLAGS"),
        ("fc", "FCFLAGS"),
        ("ld", "LDFLAGS"),
    ] {
        let flags = call(interp, vec!["get_canonical_archflags".into(), tool.into()]);
        let flags = match flags {
            Ok(flags) => flags,
            Err(_) if tool != "cc" => continue,
            Err(err) => return Err(err),
        };
        append(interp, key, flags.as_list()?.iter().cloned())?;
    }
    if architecture::universal_enabled(interp)? {
        let args = interp.scalar("configure.universal_args")?;
        let mut command = vec!["configure.pre_args-append".into()];
        command.extend(args.as_list()?.iter().cloned());
        call(interp, command)?;
    } else {
        for key in [
            "CFLAGS",
            "CXXFLAGS",
            "OBJCFLAGS",
            "OBJCXXFLAGS",
            "FFLAGS",
            "F90FLAGS",
            "FCFLAGS",
            "LDFLAGS",
        ] {
            for option in ["march", "mtune"] {
                let value = interp.scalar(&format!("configure.{option}"))?;
                if !value.as_str().is_empty() {
                    append(interp, key, [format!("-{option}={value}").into()])?;
                }
            }
        }
    }
    Ok(Value::empty())
}
