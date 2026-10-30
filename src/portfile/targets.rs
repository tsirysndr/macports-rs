//! Deferred PortSystem target overrides and ordered pre/post hooks.
use super::*;
use molt::{MoltResult, check_args};

pub(super) fn install(interp: &mut Interp, id: ContextID) -> MoltResult {
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
        // Current PortSystem target metadata. Archive/unarchive are retained
        // legacy script declarations, not standalone targets in current base.
        if !matches!(phase, "archive" | "unarchive") {
            let name = format!("org.macports.{phase}");
            let handle =
                ditems::new_target(interp, id, &name, &format!("port{phase}::{phase}_main"))?;
            interp.set_scalar(&name, handle.clone())?;
            interp
                .context::<State>(id)
                .phase_items
                .insert(phase.into(), handle.clone());
            ditems::set(interp, id, &handle, "provides", list([phase.into()]));
            let requires = match phase {
                "fetch" | "deactivate" | "uninstall" | "clean" => "main",
                "checksum" => "main fetch",
                "extract" => "main fetch checksum",
                "patch" => "main fetch checksum extract",
                "configure" => "main fetch checksum extract patch",
                "build" => "main fetch checksum extract patch configure",
                "destroot" => "main fetch checksum extract patch configure build",
                "test" => "main fetch checksum extract patch configure build destroot",
                "install" => {
                    "main archivefetch fetch checksum extract patch configure build destroot"
                }
                "activate" => {
                    "main archivefetch fetch checksum extract patch configure build destroot install"
                }
                _ => unreachable!(),
            };
            ditems::set(interp, id, &handle, "requires", requires.into());
            ditems::set(
                interp,
                id,
                &handle,
                "runpkg",
                format!("port{phase}_run").into(),
            );
            if phase != "patch" {
                ditems::set(
                    interp,
                    id,
                    &handle,
                    "prerun",
                    format!("port{phase}::{phase}_start").into(),
                );
            }
            if matches!(phase, "configure" | "destroot" | "activate") {
                ditems::set(
                    interp,
                    id,
                    &handle,
                    "postrun",
                    format!("port{phase}::{phase}_finish").into(),
                );
            }
            if phase == "fetch" {
                ditems::set(interp, id, &handle, "init", "portfetch::fetch_init".into());
            }
            if matches!(
                phase,
                "install" | "activate" | "deactivate" | "uninstall" | "clean"
            ) {
                ditems::set(interp, id, &handle, "runtype", "always".into());
                if phase != "install" {
                    ditems::set(interp, id, &handle, "state", "no".into());
                }
            }
        }
        interp.add_context_command(phase, target, id);
        for prefix in ["pre-", "post-"] {
            interp.add_context_command(&format!("{prefix}{phase}"), hook, id);
        }
    }
    Ok(Value::empty())
}

fn target(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "body")?;
    if let Some(handle) = interp
        .context::<State>(id)
        .phase_items
        .get(argv[0].as_str())
        .cloned()
    {
        let index = ditems::field(interp, id, &handle, "proc")?.as_list()?.len();
        let ident = ditems::field(interp, id, &handle, "name")?;
        let name = format!("proc-{ident}-{}-{index}", argv[0]);
        wrapper(interp, &name, argv[1].as_str())?;
        ditems::set(interp, id, &handle, "procedure", name.into());
        interp.add_context_command(&format!("do-{}", argv[0]), commands::unsupported, id);
    }
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
    let (position, phase) = argv[0].as_str().split_once('-').unwrap();
    if let Some(handle) = interp.context::<State>(id).phase_items.get(phase).cloned() {
        let mut hooks = ditems::field(interp, id, &handle, position)?
            .as_list()?
            .to_vec();
        let ident = ditems::field(interp, id, &handle, "name")?;
        let name = format!("proc-{position}-{ident}-{phase}-{}", hooks.len());
        wrapper(interp, &name, argv[1].as_str())?;
        hooks.push(name.into());
        ditems::set(interp, id, &handle, position, Value::from(hooks));
    }
    interp.context::<State>(id).hooks.push(Hook {
        phase: argv[0].to_string(),
        body: argv[1].to_string(),
    });
    Ok(Value::empty())
}

fn wrapper(interp: &mut Interp, name: &str, body: &str) -> MoltResult {
    let user = format!("user{name}");
    ditems::userproc(interp, &user, body)?;
    // Serialize both the command and its script argument as Tcl lists: target
    // names remain data even when they contain whitespace or metacharacters.
    let script = Value::from(vec![Value::from(user)]);
    let argument = Value::from(vec![script]);
    let body = format!("if {{[catch {argument} result]}} {{return -code error $result}}; return 0");
    interp.eval_value(&Value::from(vec![
        "proc".into(),
        name.into(),
        "name".into(),
        body.into(),
    ]))
}
