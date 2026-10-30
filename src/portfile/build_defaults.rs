//! Deferred build-environment defaults, installed when execution is enabled.
use super::*;
use molt::{MoltResult, check_args};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command(
        "portconfigure::configure_get_developer_dir",
        developer_dir,
        id,
    );
    interp.add_context_command("portmain::use_xcode", use_xcode, id);
}

pub(super) fn initialize(interp: &mut Interp, id: ContextID) -> MoltResult {
    // Do not resolve these while taking a metadata snapshot: workpath and
    // developer_dir belong to the execution configuration. Explicit Portfile
    // values and command-line overrides survive default registration.
    for (name, value) in [
        ("worksrcdir", "$distname"),
        ("worksrcpath", "[file join $workpath [join $worksrcdir]]"),
        ("filesdir", "files"),
        ("filespath", "[file join $portpath [join $filesdir]]"),
        ("destdir", "destroot"),
        ("destpath", "${workpath}/${destdir}"),
        ("destroot", "$destpath"),
        ("compiler.cpath", "${prefix}/include"),
        ("compiler.library_path", "${prefix}/lib"),
        ("compiler.log_verbose_output", "yes"),
        ("use_xcode", "[portmain::use_xcode]"),
        (
            "configure.developer_dir",
            "[portconfigure::configure_get_developer_dir]",
        ),
        ("configure.cmd", "./configure"),
        ("configure.dir", "$worksrcpath"),
        ("build.dir", "$worksrcpath"),
        ("build.cmd", "[portbuild::build_getmaketype]"),
        ("portutil::autoconf::make_path", ""),
        ("portutil::autoconf::bsdmake_path", ""),
        ("portutil::autoconf::gnumake_path", ""),
        ("test.asroot", "no"),
        ("test.ignore_archs", "no"),
        ("test.target", "test"),
        ("test.pre_args", "${test.target}"),
        ("test.dir", "${build.dir}"),
        ("test.cmd", "${build.cmd}"),
        ("destroot.dir", "${build.dir}"),
        ("destroot.cmd", "${build.cmd}"),
    ] {
        if !interp.var_exists(&name.into()) {
            defaults::register(interp, id, name, value)?;
        }
    }
    Ok(Value::empty())
}

fn developer_dir(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| {
        let use_xcode = if interp.var_exists(&"use_xcode".into()) {
            interp.scalar("use_xcode")?.as_bool()?
        } else {
            false
        };
        if use_xcode {
            interp.scalar("developer_dir")
        } else {
            Ok("/Library/Developer/CommandLineTools".into())
        }
    })
}

fn use_xcode(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| {
        // Match PortSystem's short-circuit expression, including older macOS
        // installations where libxcselect is the CLT-layout signal.
        if interp.scalar("build.type")?.as_str() == "xcode" {
            return Ok(true.into());
        }
        let modern_clt = filesystem::predicate("exists", "/usr/lib/libxcselect.dylib")
            || interp.scalar("os.major")?.as_int()? >= 20;
        Ok((!modern_clt
            || !filesystem::predicate(
                "executable",
                "/Library/Developer/CommandLineTools/usr/bin/make",
            ))
        .into())
    })
}
