//! Lazy build settings using explicit host/configuration inputs.
use super::*;
use molt::{MoltResult, check_args};

pub(super) fn install(interp: &mut Interp, id: ContextID) -> MoltResult {
    for name in ["build.asroot", "build.mem_per_job", "use_parallel_build"] {
        commands::register(interp, id, name);
    }
    interp.add_context_command("portbuild::build_getjobs", jobs, id);
    interp.add_context_command("portconfigure::choose_supported_archs", architectures, id);
    for (name, value) in [
        ("buildmakejobs", "0"),
        ("build.asroot", "no"),
        ("build.mem_per_job", "1024"),
        ("use_parallel_build", "yes"),
        ("build.target", "all"),
        ("build.type", "default"),
        ("build.jobs", "[portbuild::build_getjobs]"),
        ("macosx_sdk_version", ""),
        ("configure.sdk_version", "${macosx_sdk_version}"),
        ("build_arch", ""),
        ("universal_archs", ""),
        ("supported_archs", ""),
        (
            "configure.build_arch",
            "[portconfigure::choose_supported_archs $build_arch $supported_archs ${configure.sdk_version}]",
        ),
        (
            "configure.universal_archs",
            "[portconfigure::choose_supported_archs $universal_archs $supported_archs ${configure.sdk_version}]",
        ),
    ] {
        defaults::register(interp, id, name, value)?;
    }
    Ok(Value::empty())
}

fn jobs(interp: &mut Interp, _: ContextID, _: &[Value]) -> MoltResult {
    interp.with_global_scope(|interp| {
        if !interp.scalar("use_parallel_build")?.as_bool()? {
            return Ok(1.into());
        }
        let setting = interp.scalar("buildmakejobs")?;
        let mut jobs = setting.as_int().unwrap_or(1);
        if setting.as_float().ok() == Some(0.0) {
            // Match native fallback behavior when the caller has no host probe.
            jobs = optional(interp, "port_rs.active_cpus")?
                .and_then(|v| v.as_int().ok())
                .unwrap_or(2);
            if let Some(bytes) =
                optional(interp, "port_rs.memory_bytes")?.and_then(|v| v.as_float().ok())
            {
                let per_job = interp.scalar("build.mem_per_job")?;
                if let Ok(mib) = per_job.as_float() {
                    let limit = (bytes / (mib * 1024.0 * 1024.0)).trunc() + 1.0;
                    if limit.is_finite() && limit < jobs as f64 {
                        jobs = limit as i64;
                    }
                }
            }
        }
        Ok(jobs.max(1).into())
    })
}
fn optional(interp: &mut Interp, name: &str) -> Result<Option<Value>, Exception> {
    if interp.var_exists(&Value::from(name)) {
        interp.scalar(name).map(Some)
    } else {
        Ok(None)
    }
}

fn architectures(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 4, 4, "archs supported_archs sdk_version")?;
    let requested = argv[1].as_list()?;
    let supported = argv[2].as_list()?;
    let sdk = argv[3].as_str();
    let permitted = if !sdk.is_empty() {
        let at_least = |version| !crate::registry::version::compare(sdk, version).is_lt();
        let sdk_archs: &[&str] = if at_least("11") {
            &["arm64", "x86_64"]
        } else if at_least("10.14") {
            &["x86_64"]
        } else if at_least("10.7") {
            &["x86_64", "i386"]
        } else if at_least("10.6") {
            &["x86_64", "i386", "ppc"]
        } else if at_least("10.5") {
            &["x86_64", "i386", "ppc", "ppc64"]
        } else {
            &["i386", "ppc", "ppc64"]
        };
        sdk_archs
            .iter()
            .filter(|arch| supported.is_empty() || supported.iter().any(|s| s.as_str() == **arch))
            .map(|arch| (*arch).to_owned())
            .collect::<BTreeSet<_>>()
    } else if supported.is_empty() {
        return Ok(argv[1].clone());
    } else {
        supported.iter().map(ToString::to_string).collect()
    };
    let mut result = Vec::new();
    for arch in requested.iter() {
        let arch = arch.as_str();
        let demotion = match arch {
            "arm64" => "x86_64",
            "x86_64" => "i386",
            "ppc64" | "i386" => "ppc",
            _ => "",
        };
        let chosen = if permitted.contains(arch) {
            arch
        } else if permitted.contains(demotion) {
            demotion
        } else {
            continue;
        };
        if !result.iter().any(|a| a == chosen) {
            result.push(chosen.to_owned());
        }
    }
    Ok(list(result))
}
