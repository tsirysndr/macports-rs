//! Build command execution once the host has supplied build/configuration inputs.
use super::*;
use molt::MoltResult;
use std::{path::Path, sync::LazyLock};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    commands::register(interp, id, "build.jobs_arg");
    interp.add_context_command("portbuild::build_getargs", args, id);
    interp.add_context_command("portbuild::build_getjobsarg", jobs_arg, id);
    interp.add_context_command("portbuild::build_main", main, id);
}
fn args(interp: &mut Interp, _: ContextID, _: &[Value]) -> MoltResult {
    interp.with_global_scope(|interp| {
        let kind = interp.scalar("build.type")?;
        let platform = interp.scalar("os.platform")?;
        let command = interp.scalar("build.cmd")?;
        let target = interp.scalar("build.target")?;
        static MAKE: LazyLock<regex::Regex> =
            LazyLock::new(|| regex::Regex::new(r"^(/\S+/|)(g|gnu|)make(\s+.*|)$").unwrap());
        if ((kind.as_str() == "default" && platform.as_str() != "freebsd")
            || kind.as_str() == "gnu")
            && MAKE.is_match(command.as_str())
        {
            Ok(format!("-w {target}").into())
        } else {
            Ok(target)
        }
    })
}
fn jobs_arg(interp: &mut Interp, _: ContextID, _: &[Value]) -> MoltResult {
    interp.with_global_scope(|interp| {
        let command = interp.scalar("build.cmd")?;
        let words = command.as_list()?;
        let program = words.first().map(Value::as_str).unwrap_or("");
        let name = Path::new(program)
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("");
        if !interp.var_exists(&Value::from("build.jobs"))
            || !(name.ends_with("make") || matches!(name, "ninja" | "scons"))
        {
            return Ok(Value::empty());
        }
        let jobs = interp.scalar("build.jobs")?.as_int().unwrap_or(0);
        if jobs < 1 || (name != "ninja" && jobs < 2) {
            return Ok(Value::empty());
        }
        Ok(format!(" -j{jobs}").into())
    })
}
fn main(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        let original = interp.scalar("build.cmd")?;
        if original.as_str().is_empty() {
            return Err(error("No build command found"));
        }
        let parallel = if interp.var_exists(&Value::from("build.jobs_arg")) {
            interp.scalar("build.jobs_arg")?
        } else {
            jobs_arg(interp, id, &[])?
        };
        let missing_pre_args = !interp.var_exists(&Value::from("build.pre_args"));
        if missing_pre_args {
            let value = args(interp, id, &[])?;
            interp.set_scalar("build.pre_args", value)?;
        }
        interp.set_scalar("build.cmd", format!("{original}{parallel}").into())?;
        // The Rust runner already renders live phase output. Native progress
        // callback parsing is separate and has not been implemented yet.
        let result = interp.eval_value(&Value::from(vec!["command_exec".into(), "build".into()]));
        interp.set_scalar("build.cmd", original)?;
        if missing_pre_args {
            interp.unset("build.pre_args");
        }
        result?;
        Ok(0.into())
    })
}
