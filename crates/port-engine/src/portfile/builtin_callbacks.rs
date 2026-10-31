//! PortSystem's built-in post-evaluation callbacks, which add dependencies the
//! Portfile implies (extract tools, MacPorts compilers, make flavours).
use super::*;
use molt::{MoltResult, check_args};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    for (name, command) in [
        (
            "portextract::add_extract_deps",
            extract_deps as molt::CommandFunc,
        ),
        (
            "portbuild::add_automatic_buildsystem_dependencies",
            buildsystem_deps,
        ),
        (
            "portconfigure::add_automatic_compiler_dependencies",
            compiler_deps,
        ),
        (
            "portconfigure::add_compiler_port_dependencies",
            compiler_port_deps,
        ),
    ] {
        interp.add_context_command(name, command, id);
    }
    interp.context::<State>(id).callbacks.extend(
        // In upstream package load order: portextract, portconfigure, portbuild.
        [
            "portextract::add_extract_deps",
            "portconfigure::add_automatic_compiler_dependencies",
            "portbuild::add_automatic_buildsystem_dependencies",
        ]
        .map(String::from),
    );
}

fn call(interp: &mut Interp, words: &[&str]) -> MoltResult {
    interp.eval_value(&Value::from(
        words
            .iter()
            .map(|word| Value::from(*word))
            .collect::<Vec<_>>(),
    ))
}

fn enabled(interp: &mut Interp, name: &str) -> Result<bool, Exception> {
    Ok(interp.var_exists(&name.into()) && interp.scalar(name)?.as_bool().unwrap_or(false))
}

fn add(interp: &mut Interp, field: &str, dependency: &str, replace: bool) -> Result<(), Exception> {
    if replace {
        call(interp, &[&format!("{field}-delete"), dependency])?;
    }
    call(interp, &[&format!("{field}-append"), dependency])?;
    Ok(())
}

fn extract_deps(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| {
        let dependency = if enabled(interp, "use_bzip2")? {
            call(interp, &["macports::binaryInPath", "lbzip2"])
                .ok()
                .map(|_| "bin:lbzip2:lbzip2")
        } else if enabled(interp, "use_lzma")? {
            Some("bin:lzma:xz")
        } else if enabled(interp, "use_xz")? {
            Some("bin:xz:xz")
        } else if enabled(interp, "use_zip")? {
            Some("bin:unzip:unzip")
        } else if enabled(interp, "use_7z")? {
            Some("bin:7za:p7zip")
        } else if enabled(interp, "use_lzip")? {
            Some("bin:lzip:lzip")
        } else {
            None
        };
        if let Some(dependency) = dependency {
            add(interp, "depends_extract", dependency, false)?;
        }
        Ok(Value::empty())
    })
}

/// Host-dependent callbacks are no-ops for library sessions without host facts.
fn hosted(interp: &mut Interp) -> bool {
    interp.var_exists(&"os.platform".into()) && interp.var_exists(&"os.major".into())
}

fn buildsystem_deps(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| {
        if !hosted(interp) || !enabled(interp, "build.type.add_deps")? {
            return Ok(Value::empty());
        }
        let kind = interp.scalar("build.type")?;
        let platform = interp.scalar("os.platform")?;
        match (kind.as_str(), platform.as_str()) {
            ("bsd", "darwin") => add(interp, "depends_build", "bin:bsdmake:bsdmake", true)?,
            ("gnu", "freebsd") => add(interp, "depends_build", "bin:gmake:gmake", true)?,
            _ => {}
        }
        Ok(Value::empty())
    })
}

fn compiler_deps(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| {
        if !hosted(interp) || !enabled(interp, "configure.compiler.add_deps")? {
            return Ok(Value::empty());
        }
        let compiler = interp.scalar("configure.compiler")?;
        let is_port = call(
            interp,
            &["portconfigure::compiler_is_port", compiler.as_str()],
        )?;
        if is_port.as_bool()? {
            call(
                interp,
                &[
                    "portconfigure::add_compiler_port_dependencies",
                    compiler.as_str(),
                ],
            )?;
        }
        if enabled(interp, "compiler.require_fortran")? {
            let fortran = call(
                interp,
                &[
                    "portconfigure::configure_get_compiler",
                    "fc",
                    compiler.as_str(),
                ],
            )?;
            if fortran.as_str().is_empty() {
                let chosen = call(interp, &["portconfigure::configure_get_fortran_compiler"])?;
                call(
                    interp,
                    &[
                        "portconfigure::add_compiler_port_dependencies",
                        chosen.as_str(),
                    ],
                )?;
            }
        }
        Ok(Value::empty())
    })
}

fn compiler_port_deps(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "compiler")?;
    let compiler = argv[1].to_string();
    interp.with_global_scope(|interp| {
        let port = compilers::compiler_port(&compiler);
        let mpi = regex::Regex::new(
            r"^macports-(mpich|openmpi)-(default|clang|gcc)(?:-(\d+(?:\.\d+)?))?$",
        )
        .unwrap();
        if compiler == "apple-gcc-4.0" {
            add(interp, "depends_lib", &format!("port:{port}"), true)?;
        } else if let Some(captures) = mpi.captures(&compiler) {
            let package = if &captures[1] == "openmpi" {
                "ompi.pc".to_string()
            } else {
                format!("{}.pc", &captures[1])
            };
            add(
                interp,
                "depends_lib",
                &format!("path:lib/{port}/pkgconfig/{package}:{port}"),
                true,
            )?;
        } else {
            add(interp, "depends_build", &format!("port:{port}"), true)?;
            call(interp, &["license_noconflict-append", port.as_str()])?;
            let gcc = regex::Regex::new(r"^macports-gcc-(\d+(?:\.\d+)?)?$").unwrap();
            let clang = regex::Regex::new(r"^macports-clang(?:-(\d+(?:\.\d+)?))$").unwrap();
            if let Some(captures) = gcc.captures(&compiler) {
                let version = captures.get(1).map(|m| m.as_str()).unwrap_or("");
                let platform = interp.scalar("os.platform")?;
                let major = interp.scalar("os.major")?.as_int().unwrap_or(0);
                let main = if platform.as_str() == "darwin" && major < 10 {
                    "7"
                } else {
                    "13"
                };
                let older = |a: &str, b: &str| crate::registry::compare_versions(a, b).is_lt();
                let mut libraries = vec!["path:share/doc/libgcc/README:libgcc".to_string()];
                if older(version, "4.6") {
                    libraries.push("port:libgcc45".into());
                } else if older(version, "7") {
                    libraries.push("port:libgcc6".into());
                } else if older(version, main) {
                    libraries.push(format!("port:libgcc{version}"));
                }
                for library in libraries {
                    add(interp, "depends_lib", &library, true)?;
                }
            } else if clang.is_match(&compiler) {
                let stdlib = interp.scalar("configure.cxx_stdlib")?;
                let major = interp.scalar("os.major")?.as_int().unwrap_or(0);
                if stdlib.as_str() == "macports-libstdc++" {
                    add(
                        interp,
                        "depends_lib",
                        "path:lib/libgcc/libgcc_s.1.dylib:libgcc",
                        true,
                    )?;
                } else if stdlib.as_str() == "libc++" && major < 11 {
                    add(interp, "depends_lib", "port:libcxx", true)?;
                }
                if !interp
                    .scalar("compiler.openmp_version")?
                    .as_str()
                    .is_empty()
                {
                    add(interp, "depends_lib", "port:libomp", true)?;
                }
            }
        }
        let supported = call(
            interp,
            &["portconfigure::arch_flag_supported", compiler.as_str()],
        )?;
        if supported.as_bool()? {
            call(interp, &["depends_skip_archcheck-delete", port.as_str()])?;
            call(interp, &["depends_skip_archcheck-append", port.as_str()])?;
        }
        Ok(Value::empty())
    })
}
