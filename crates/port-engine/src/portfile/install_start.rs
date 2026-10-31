//! Install startup and account declaration dispatch, matching portinstall_run.
use super::*;
use molt::{MoltResult, check_args};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portinstall::install_start", start, id);
    interp.add_context_command("handle_add_users", users, id);
}
fn invoke(interp: &mut Interp, words: Vec<Value>) -> MoltResult {
    interp.eval_value(&Value::from(words))
}
fn start(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        let label = format!(
            "Installing {} @{}_{}{}",
            interp.scalar("subport")?,
            interp.scalar("version")?,
            interp.scalar("revision")?,
            interp.scalar("portvariants")?
        );
        let prefix = interp.scalar("UI_PREFIX")?;
        interp
            .context::<State>(id)
            .progress
            .clone()
            .unwrap_or_else(|| crate::progress::Progress::new(false))
            .phase_with_prefix(prefix.as_str(), &label);
        let prefix = interp.scalar("prefix_frozen")?;
        if !filesystem::predicate("writable", prefix.as_str()) || crate::privileges::dropped() {
            crate::privileges::elevate("install").map_err(|err| error(err.to_string()))?;
        }
        invoke(interp, vec!["handle_add_users".into()])
    })
}
fn users(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        let values = interp.scalar("add_users")?.as_list()?;
        let mut current = String::new();
        let mut arguments: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for value in values.iter() {
            let text = value.as_str();
            if text.contains('=') && !current.is_empty() {
                let mut parts = text.split('=');
                let key = parts.next().unwrap();
                let value = if key == "group" {
                    // Tcl split/lindex intentionally uses only the second field.
                    let group = parts.next().unwrap_or("");
                    invoke(interp, vec!["addgroup".into(), group.into()])?;
                    let gid = invoke(interp, vec!["existsgroup".into(), group.into()])?;
                    format!("gid={gid}").into()
                } else {
                    value.clone()
                };
                arguments.entry(current.clone()).or_default().push(value);
            } else {
                current = text.into();
            }
        }
        // Upstream iterates an unordered Tcl array; do not promise user order.
        for (name, args) in arguments {
            let mut words = vec!["adduser".into(), name.into()];
            words.extend(args);
            invoke(interp, words)?;
        }
        Ok(Value::empty())
    })
}
