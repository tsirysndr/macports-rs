//! Deferred PortSystem target overrides and ordered pre/post hooks.
use super::*;
use molt::{MoltResult, check_args};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    for phase in [
        "fetch",
        "checksum",
        "extract",
        "patch",
        "configure",
        "build",
        "destroot",
        "install",
        "activate",
        "deactivate",
        "uninstall",
        "test",
        "archive",
        "unarchive",
        "clean",
    ] {
        interp.add_context_command(phase, target, id);
        for prefix in ["pre-", "post-"] {
            interp.add_context_command(&format!("{prefix}{phase}"), hook, id);
        }
    }
}

fn target(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "body")?;
    // target_provides replaces the main procedure on each declaration, while
    // pre/post declarations append. Empty overrides intentionally disable the
    // built-in phase and must remain distinguishable from an absent override.
    interp
        .context::<State>(id)
        .targets
        .insert(argv[0].to_string(), argv[1].to_string());
    Ok(Value::empty())
}

fn hook(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "body")?;
    interp.context::<State>(id).hooks.push(Hook {
        phase: argv[0].to_string(),
        body: argv[1].to_string(),
    });
    Ok(Value::empty())
}
