//! PortSystem test-command execution; staged Mach-O validation is still pending.
use super::*;
use molt::MoltResult;

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    for name in ["test.asroot", "test.ignore_archs"] {
        commands::register(interp, id, name);
    }
    interp.add_context_command("porttest::test_main", main, id);
    interp.add_context_command("porttest::test_archs", architectures, id);
}

fn architectures(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    molt::check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| {
        if interp.scalar("os.platform")?.as_str() != "darwin"
            || interp.scalar("test.ignore_archs")?.as_bool()?
        {
            return Ok(Value::empty());
        }
        // Do not report a passing test phase while silently skipping the
        // mandatory staged-binary checks performed by native PortSystem.
        commands::unsupported(interp, id, argv)
    })
}

fn main(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        // Invoke by name so a Portfile's replacement procedure is honored.
        interp.eval_value(&Value::from(vec![Value::from("porttest::test_archs")]))?;
        let run = interp.var_exists(&"test.run".into()) && interp.scalar("test.run")?.as_bool()?;
        if run {
            interp.eval_value(&Value::from(vec!["command_exec".into(), "test".into()]))?;
        }
        Ok(0.into())
    })
}
