//! Deferred PortSystem target overrides and ordered pre/post hooks.
use super::*;
use molt::{MoltResult, check_args};

#[derive(Clone)]
struct Binding {
    state: ContextID,
    handle: Option<Value>,
    phase: String,
    ident: String,
    original: String,
    position: Option<&'static str>,
}

pub(super) fn install(interp: &mut Interp, id: ContextID) -> MoltResult {
    interp.add_context_command("target_provides", provides, id);
    // PortSystem's main target is stateless, has no runtime package or
    // prerequisites, and its native procedure simply returns zero.
    interp.add_context_command("portmain::main", main, id);
    let handle = ditems::new_target(interp, id, "org.macports.main", "portmain::main")?;
    interp.set_scalar("org.macports.main", handle.clone())?;
    ditems::set(interp, id, &handle, "provides", list(["main".into()]));
    ditems::set(interp, id, &handle, "state", "no".into());
    register(interp, id, "main", Some(handle))?;
    for phase in [
        "archivefetch",
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
        let handle = if !matches!(phase, "archive" | "unarchive") {
            let name = format!("org.macports.{phase}");
            let handle =
                ditems::new_target(interp, id, &name, &format!("port{phase}::{phase}_main"))?;
            interp.set_scalar(&name, handle.clone())?;
            ditems::set(interp, id, &handle, "provides", list([phase.into()]));
            let requires = match phase {
                "archivefetch" | "fetch" | "deactivate" | "uninstall" | "clean" => "main",
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
                "archivefetch" | "install" | "activate" | "deactivate" | "uninstall" | "clean"
            ) {
                ditems::set(interp, id, &handle, "runtype", "always".into());
                if !matches!(phase, "archivefetch" | "install") {
                    ditems::set(interp, id, &handle, "state", "no".into());
                }
            }
            Some(handle)
        } else {
            None
        };
        register(interp, id, phase, handle)?;
    }
    Ok(Value::empty())
}

fn main(_: &mut Interp, _: ContextID, _: &[Value]) -> MoltResult {
    Ok(0.into())
}

fn register(
    interp: &mut Interp,
    state: ContextID,
    phase: &str,
    handle: Option<Value>,
) -> MoltResult {
    let (ident, original) = if let Some(handle) = &handle {
        (
            ditems::field(interp, state, handle, "name")?.to_string(),
            ditems::field(interp, state, handle, "procedure")?.to_string(),
        )
    } else {
        (String::new(), String::new())
    };
    let binding = Binding {
        state,
        handle,
        phase: phase.into(),
        ident,
        original,
        position: None,
    };
    // Existing phase commands retain their original binding. Pre/post commands
    // are replaced on each target_provides, just as in native PortSystem.
    if !interp.has_command(phase) {
        let id = interp.save_context(binding.clone());
        interp.add_context_command(phase, target, id);
    }
    for position in ["pre", "post"] {
        let mut binding = binding.clone();
        binding.position = Some(position);
        let id = interp.save_context(binding);
        let command = format!("{position}-{phase}");
        if interp.has_command(&command) {
            interp.remove_command(&command);
        }
        interp.add_context_command(&command, hook, id);
    }
    Ok(Value::empty())
}

fn provides(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 0, "ditem ?target ...?")?;
    for phase in &argv[2..] {
        register(interp, id, phase.as_str(), Some(argv[1].clone()))?;
    }
    let mut values = ditems::field(interp, id, &argv[1], "provides")?
        .as_list()?
        .to_vec();
    values.extend_from_slice(&argv[2..]);
    let values = Value::from(values);
    ditems::set(interp, id, &argv[1], "provides", values.clone());
    Ok(values)
}

fn target(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "body")?;
    let binding = interp.context::<Binding>(id).clone();
    if let Some(handle) = &binding.handle {
        let index = ditems::field(interp, binding.state, handle, "proc")?
            .as_list()?
            .len();
        let name = format!("proc-{}-{}-{index}", binding.ident, binding.phase);
        wrapper(interp, &name, argv[1].as_str())?;
        ditems::set(interp, binding.state, handle, "procedure", name.into());
        // Original procedure and phase are data, even when their names contain
        // whitespace, semicolons, or substitutions. Capture at registration time.
        let body = Value::from(vec![
            Value::from(binding.original),
            Value::from(binding.phase.clone()),
        ]);
        interp.eval_value(&Value::from(vec![
            "proc".into(),
            format!("do-{}", binding.phase).into(),
            Value::empty(),
            body,
        ]))?;
    }
    interp
        .context::<State>(binding.state)
        .targets
        .insert(binding.phase, argv[1].to_string());
    Ok(Value::empty())
}

fn hook(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "body")?;
    let binding = interp.context::<Binding>(id).clone();
    let position = binding.position.expect("hook binding has a position");
    if let Some(handle) = &binding.handle {
        let mut hooks = ditems::field(interp, binding.state, handle, position)?
            .as_list()?
            .to_vec();
        let name = format!(
            "proc-{position}-{}-{}-{}",
            binding.ident,
            binding.phase,
            hooks.len()
        );
        wrapper(interp, &name, argv[1].as_str())?;
        hooks.push(name.into());
        ditems::set(interp, binding.state, handle, position, Value::from(hooks));
    }
    interp.context::<State>(binding.state).hooks.push(Hook {
        phase: format!("{position}-{}", binding.phase),
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
