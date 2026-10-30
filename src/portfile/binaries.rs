//! PortSystem executable lookup using the session environment, never host PATH.
use super::*;
use molt::{MoltResult, check_args};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    for name in ["findBinary", "macports::findBinary"] {
        interp.add_context_command(name, find, id);
    }
    interp.add_context_command("macports::binaryInPath", in_path, id);
    interp.add_context_command("portbuild::build_getmaketype", make, id);
}
fn find(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 3, "prog ?autoconf_hint?")?;
    interp.with_global_scope(|interp| {
        if let Some(hint) = argv.get(2)
            && !hint.as_str().is_empty()
            && filesystem::predicate("executable", hint.as_str())
        {
            return Ok(hint.clone());
        }
        lookup(interp, argv[1].as_str()).map_err(|err| {
            error(format!(
                "{} or at its MacPorts configuration time location, did you move it?",
                err.value()
            ))
        })
    })
}
fn in_path(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "prog")?;
    interp.with_global_scope(|interp| lookup(interp, argv[1].as_str()))
}
fn lookup(interp: &mut Interp, program: &str) -> MoltResult {
    let path = interp.element("env", "PATH")?;
    for directory in path.as_str().split(':') {
        let candidate = paths::join([directory, program]);
        if filesystem::predicate("executable", &candidate) {
            return Ok(candidate.into());
        }
    }
    Err(error(format!(
        "Failed to locate '{program}' in path: '{path}'"
    )))
}
fn make(interp: &mut Interp, id: ContextID, _: &[Value]) -> MoltResult {
    interp.with_global_scope(|interp| {
        let kind = interp.scalar("build.type")?;
        let result = (|| {
            let (program, hint) = if kind.as_str() == "default" {
                ("make", "make_path")
            } else {
                let platform = if matches!(kind.as_str(), "bsd" | "gnu" | "pbx" | "xcode") {
                    interp.scalar("os.platform")?
                } else {
                    Value::empty()
                };
                match kind.as_str() {
                    "bsd" => match platform.as_str() {
                        "darwin" => ("bsdmake", "bsdmake_path"),
                        "freebsd" => ("make", "make_path"),
                        _ => ("pmake", "bsdmake_path"),
                    },
                    "gnu" => match platform.as_str() {
                        "darwin" => ("gnumake", "gnumake_path"),
                        "linux" => ("make", "make_path"),
                        _ => ("gmake", "gnumake_path"),
                    },
                    "pbx" | "xcode" => {
                        if platform.as_str() != "darwin" {
                            return Err(error(format!("This port requires 'xcodebuild', which is not available on {platform}.")));
                        }
                        let command = interp.scalar("xcodebuildcmd")?;
                        return if command.as_str() != "none" { Ok(command) }
                        else { Err(error("xcodebuild was not found on this system!")) };
                    }
                    _ => {
                        eprintln!("Warning: Unknown build.type {kind}, using 'gnumake'");
                        ("gnumake", "gnumake_path")
                    }
                }
            };
            let hint = interp.scalar(&format!("portutil::autoconf::{hint}"))?;
            find(interp, id, &["findBinary".into(), program.into(), hint])
        })();
        match result {
            Ok(value) => Ok(value),
            Err(err) => {
                eprintln!("Warning: {}", err.value());
                eprintln!("Warning: Unable to find build command for build.type '{kind}'");
                Ok(Value::empty())
            }
        }
    })
}
