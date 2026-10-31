//! Configure startup validation after host compiler selection.
use super::*;
use molt::MoltResult;
use std::sync::LazyLock;

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portconfigure::configure_start", start, id);
}
fn valid_compiler(compiler: &str) -> bool {
    static COMPILER: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(concat!(
        r"\A(?:apple-gcc-4\.[02]|cc|clang|gcc|gcc-(?:3\.3|4\.[02])|llvm-gcc-4\.2|",
        r"macports-(?:clang|gcc)(?:-\d+(?:\.\d+)?|-devel)?|macports-llvm-gcc-4\.2|macports-g95|",
        r"macports-(?:mpich|openmpi)-(?:default|clang|(?:clang|gcc)-\d+(?:\.\d+)?))\z"
    )).unwrap()
    });
    COMPILER.is_match(compiler)
}
fn start(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    phase_start::notice(interp, id, argv, "Configuring")?;
    interp.with_global_scope(|interp| {
        let compiler = interp.scalar("configure.compiler")?;
        if !valid_compiler(compiler.as_str()) {
            return Err(error(format!(
                "Invalid value for configure.compiler: {compiler}"
            )));
        }
        phase_start::warn_compiler_fallback(interp, id)?;
        let ccache = interp.var_exists(&"configure.ccache".into())
            && interp.scalar("configure.ccache")?.as_bool()?;
        if ccache {
            // Upstream elevates outside its catch block, so a non-root caller
            // errors rather than silently disabling ccache. Process-wide
            // privilege changes need the isolated worker, not this interpreter.
            if unsafe { libc::getuid() } != 0 {
                return Err(error("MacPorts requires root privileges for this action"));
            }
            interp
                .context::<State>(id)
                .unsupported
                .insert("configure ccache privilege lifecycle".into());
            return Err(error(
                "configure ccache initialization requires the privilege worker",
            ));
        }
        Ok(Value::empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_upstream_compiler_families_and_rejects_unrecognized_spellings() {
        for compiler in [
            "apple-gcc-4.0",
            "apple-gcc-4.2",
            "cc",
            "clang",
            "gcc",
            "gcc-3.3",
            "gcc-4.0",
            "gcc-4.2",
            "llvm-gcc-4.2",
            "macports-clang",
            "macports-gcc",
            "macports-clang-18",
            "macports-clang-3.4",
            "macports-gcc-14",
            "macports-llvm-gcc-4.2",
            "macports-g95",
            "macports-mpich-default",
            "macports-openmpi-default",
            "macports-mpich-clang",
            "macports-openmpi-clang",
            "macports-mpich-clang-18",
            "macports-openmpi-clang-3.4",
            "macports-mpich-gcc-14",
            "macports-openmpi-gcc-14.1",
            "macports-clang-devel",
            "macports-gcc-devel",
        ] {
            assert!(valid_compiler(compiler), "{compiler}");
        }
        for compiler in [
            "",
            "clang-18",
            "gcc-14",
            "apple-gcc-4.1",
            "macports-mpich-gcc",
            "macports-clang-18.1.2",
            "macports-gcc-",
            "clang\n",
            "/usr/bin/clang",
            " clang",
            "macports-clang-devel extra",
        ] {
            assert!(!valid_compiler(compiler), "{compiler}");
        }
    }
}
