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

/// Build-path defaults Portfiles may reference during evaluation. They stay
/// lazy, and metadata snapshots skip them unless a Portfile assigns a value.
pub(super) fn register_paths(interp: &mut Interp, id: ContextID) -> MoltResult {
    for (name, value) in [
        (
            "portbuildpath",
            "[file join $portdbpath build [string map {/ _} $portpath]]",
        ),
        (
            "subbuildpath",
            "[file normalize [file join $portbuildpath [expr {$subport ne {} ? $subport : [file tail $portpath]}]]]",
        ),
        (
            "workpath",
            "[file normalize [file join $subbuildpath work]]",
        ),
        ("distpath", "[file join $portdbpath distfiles $dist_subdir]"),
        ("worksrcdir", "$distname"),
        ("worksrcpath", "[file join $workpath [join $worksrcdir]]"),
        ("filesdir", "files"),
        ("filespath", "[file join $portpath [join $filesdir]]"),
        ("destdir", "destroot"),
        ("destpath", "${workpath}/${destdir}"),
        ("destroot", "$destpath"),
    ] {
        if !interp.var_exists(&name.into()) {
            defaults::register(interp, id, name, value)?;
            interp
                .context::<State>(id)
                .execution_defaults
                .insert(name.into());
        }
    }
    Ok(Value::empty())
}

pub(super) fn initialize(interp: &mut Interp, id: ContextID) -> MoltResult {
    // Explicit Portfile values and command-line overrides survive default
    // registration.
    for (name, value) in [
        ("compiler.cpath", "${prefix}/include"),
        ("compiler.library_path", "${prefix}/lib"),
        ("compiler.log_verbose_output", "yes"),
        ("use_xcode", "[portmain::use_xcode]"),
        (
            "configure.developer_dir",
            "[portconfigure::configure_get_developer_dir]",
        ),
        ("use_configure", "yes"),
        ("configure.checks.implicit_function_declaration", "yes"),
        ("configure.checks.implicit_int", "yes"),
        (
            "configure.checks.incompatible_function_pointer_types",
            "yes",
        ),
        ("portconfigure::no_default_compiler_allowed", "0"),
        ("UI_PREFIX", "--->"),
        ("configure.env", ""),
        ("configure.pre_args", "--prefix=${prefix}"),
        ("configure.march", ""),
        ("configure.mtune", ""),
        ("configure.universal_args", "--disable-dependency-tracking"),
        ("autoreconf.dir", "$worksrcpath"),
        ("autoreconf.args", "--install --verbose"),
        ("automake.dir", "$worksrcpath"),
        ("automake.args", "--verbose"),
        ("autoconf.dir", "$worksrcpath"),
        ("autoconf.args", "--verbose"),
        ("xmkmf.dir", "$worksrcpath"),
        ("xmkmf.cmd", "xmkmf"),
        ("configure.cmd", "./configure"),
        ("configure.dir", "$worksrcpath"),
        ("build.dir", "$worksrcpath"),
        ("build.cmd", "[portbuild::build_getmaketype]"),
        ("portutil::autoconf::make_path", ""),
        ("portutil::autoconf::bsdmake_path", ""),
        ("portutil::autoconf::gnumake_path", ""),
        ("portutil::autoconf::bzip2_path", "/usr/bin/bzip2"),
        ("portutil::autoconf::lzma_path", ""),
        ("portutil::autoconf::xz_path", ""),
        ("portutil::autoconf::gzip_path", "/usr/bin/gzip"),
        ("portutil::autoconf::tar_command", "/usr/bin/tar"),
        ("portutil::autoconf::tar_path", "/usr/bin/tar"),
        ("portutil::autoconf::unzip_path", "/usr/bin/unzip"),
        ("portutil::autoconf::hdiutil_path", "/usr/bin/hdiutil"),
        ("portutil::autoconf::find_path", "/usr/bin/find"),
        ("portutil::autoconf::cpio_path", "/usr/bin/cpio"),
        ("portutil::autoconf::rmdir_path", "/bin/rmdir"),
        ("portutil::autoconf::patch_path", "/usr/bin/patch"),
        ("portutil::autoconf::gnupatch_path", ""),
        ("portutil::autoconf::file_path", "/usr/bin/file"),
        ("extract.asroot", "no"),
        ("extract.only", "[portextract::disttagclean $distfiles]"),
        ("extract.dir", "${workpath}"),
        ("extract.cmd", "[portextract::get_extract_cmd]"),
        ("extract.pre_args", "[portextract::get_extract_pre_args]"),
        ("extract.post_args", "[portextract::get_extract_post_args]"),
        ("extract.mkdir", "no"),
        ("extract.rename", "no"),
        ("patch.asroot", "no"),
        ("patch.dir", "${worksrcpath}"),
        ("patch.cmd", "[portpatch::build_getpatchtype]"),
        ("patch.pre_args", "-t -N -p0"),
        ("test.asroot", "no"),
        ("test.ignore_archs", "no"),
        ("test.target", "test"),
        ("test.pre_args", "${test.target}"),
        ("test.dir", "${build.dir}"),
        ("test.cmd", "${build.cmd}"),
        ("destroot.asroot", "no"),
        ("destroot.umask", "$system_options(destroot_umask)"),
        ("destroot.target", "install"),
        ("destroot.pre_args", "[portdestroot::destroot_getargs]"),
        ("destroot.post_args", "${destroot.destdir}"),
        ("destroot.destdir", "DESTDIR=${destroot}"),
        ("destroot.clean", "no"),
        ("destroot.keepdirs", ""),
        ("destroot.violate_mtree", "no"),
        ("destroot.dir", "${build.dir}"),
        ("destroot.cmd", "${build.cmd}"),
    ] {
        if !interp.var_exists(&name.into()) {
            defaults::register(interp, id, name, value)?;
        }
    }
    startupitem::initialize(interp, id)?;
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
