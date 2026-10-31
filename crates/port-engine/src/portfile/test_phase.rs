//! PortSystem test-command execution after staged architecture checks.
use super::*;
use molt::MoltResult;

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    for name in ["test.asroot", "test.ignore_archs"] {
        commands::register(interp, id, name);
    }
    interp.add_context_command("porttest::test_main", main, id);
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
