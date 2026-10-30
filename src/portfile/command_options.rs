//! PortSystem command option families and configure metadata declarations.
//! Declaring a command family never launches a process or selects a compiler.
use super::*;
use molt::MoltResult;

pub(super) fn install(interp: &mut Interp, id: ContextID) -> MoltResult {
    interp.add_context_command("commands", declare, id);
    for name in [
        "configure",
        "autoreconf",
        "automake",
        "autoconf",
        "xmkmf",
        "build",
        "destroot",
        "extract",
        "patch",
        "test",
        "bzr",
        "cvs",
        "svn",
    ] {
        family(interp, id, name)?;
    }
    for name in [
        "configure.checks.implicit_int",
        "configure.checks.incompatible_function_pointer_types",
        "configure.checks.implicit_function_declaration",
        "configure.checks.implicit_function_declaration.whitelist",
        "configure.compiler",
        "configure.compiler.add_deps",
        "configure.optflags",
        "configure.libs",
        "configure.classpath",
        "configure.cxx_stdlib",
        "configure.asroot",
        "configure.ccache",
        "configure.distcc",
        "configure.pipe",
        "configure.m32",
        "configure.m64",
        "configure.march",
        "configure.mtune",
        "configure.sdk_version",
        "configure.sdkroot",
        "configure.sysroot",
        "configure.developer_dir",
        "configure.universal_args",
        "configure.perl",
        "configure.python",
        "configure.ruby",
        "configure.install",
        "configure.awk",
        "configure.bison",
        "configure.pkg_config",
        "configure.pkg_config_path",
        "compiler.whitelist",
        "compiler.fallback",
        "compiler.c_standard",
        "compiler.cxx_standard",
        "compiler.openmp_version",
        "compiler.mpi",
        "compiler.thread_local_storage",
    ] {
        commands::register(interp, id, name);
    }
    for tool in [
        "cc", "cxx", "objc", "objcxx", "cpp", "f77", "f90", "fc", "javac",
    ] {
        commands::register(interp, id, &format!("configure.{tool}"));
    }
    for tool in ["cc", "cxx", "objc", "objcxx", "f77", "f90", "fc", "ld"] {
        commands::register(interp, id, &format!("configure.{tool}_archflags"));
    }
    for language in ["c", "cxx", "objc", "objcxx", "cpp", "ld", "f", "f90", "fc"] {
        commands::register(interp, id, &format!("configure.{language}flags"));
    }
    for language in ["c", "cxx", "objc", "objcxx", "cpp", "ld"] {
        commands::register(interp, id, &format!("configure.universal_{language}flags"));
    }
    Ok(Value::empty())
}

fn declare(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    for name in &argv[1..] {
        family(interp, id, name.as_str())?;
    }
    Ok(Value::empty())
}

fn family(interp: &mut Interp, id: ContextID, name: &str) -> MoltResult {
    if name.is_empty()
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.".contains(&c))
    {
        return Err(error("invalid command option name"));
    }
    let mut names = vec![format!("use_{name}")];
    names.extend(
        [
            "dir",
            "pre_args",
            "args",
            "post_args",
            "env",
            "nice",
            "type",
            "cmd",
        ]
        .map(|suffix| format!("{name}.{suffix}")),
    );
    // Use the same collision checks as `options` for PortGroup-created families.
    let mut args = vec![Value::from("options")];
    args.extend(names.into_iter().map(Value::from));
    commands::options(interp, id, &args)
}
