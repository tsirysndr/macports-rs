use super::*;
use molt::{MoltResult, check_args};
pub(super) const OPTIONS: &[&str] = &[
    "name",
    "version",
    "revision",
    "epoch",
    "categories",
    "maintainers",
    "license",
    "description",
    "long_description",
    "homepage",
    "platforms",
    "supported_archs",
    "compiler.blacklist",
    "configure.build_arch",
    "configure.universal_archs",
    "default_variants",
    "depends_fetch",
    "depends_extract",
    "depends_patch",
    "depends_build",
    "depends_lib",
    "depends_run",
    "depends_test",
    "checksums",
    "master_sites",
    "distfiles",
    "patchfiles",
    "worksrcdir",
    "distname",
    "configure.args",
    "configure.env",
    "configure.cflags",
    "configure.cppflags",
    "configure.ldflags",
    "build.args",
    "build.env",
    "build.target",
    "destroot.args",
    "destroot.env",
    "destroot.target",
    "conflicts",
    "replaced_by",
    "notes",
    "use_configure",
    "use_xz",
    "use_bzip2",
    "use_zip",
    "test.run",
    "test.target",
    "test.args",
    "test.env",
];
pub(super) fn register(interp: &mut Interp, id: ContextID, name: &str) {
    interp.context::<State>(id).options.insert(name.into());
    for suffix in ["", "-append", "-prepend", "-delete", "-replace"] {
        interp.add_context_command(&format!("{name}{suffix}"), option, id);
    }
}
pub(super) fn install(interp: &mut Interp, id: ContextID) {
    for (name, command) in [
        ("PortSystem", system as molt::CommandFunc),
        ("options", options),
        ("option", access),
        ("exists", exists),
        ("variant", super::variants::declare),
        ("variant_isset", super::variants::query),
        ("variant_exists", super::variants::query),
        ("variant_set", super::variants::query),
        ("variant_unset", super::variants::query),
        ("subport", subport),
    ] {
        interp.add_context_command(name, command, id);
    }
    for phase in [
        "fetch",
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
    ] {
        for prefix in ["pre-", "post-"] {
            interp.add_context_command(&format!("{prefix}{phase}"), hook, id);
        }
    }
}
pub(super) fn unsupported(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    let name = argv[0].as_str();
    interp.context::<State>(id).unsupported.insert(name.into());
    Err(error(format!("unsupported Portfile command {name}")))
}
fn system(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "version")?;
    if argv[1].as_str() != "1.0" {
        return Err(error("only PortSystem 1.0 is supported"));
    }
    Ok(Value::empty())
}
fn options(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    for name in &argv[1..] {
        let name = name.as_str();
        if name.is_empty()
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_.".contains(&c))
        {
            return Err(error("invalid option name"));
        }
        if interp
            .command_names()
            .iter()
            .any(|command| command.as_str() == name)
            && !interp.context::<State>(id).options.contains(name)
        {
            return Err(error("option would replace an existing command"));
        }
        register(interp, id, name);
    }
    Ok(Value::empty())
}
fn option(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    let command = argv[0].as_str();
    let (name, operation) = command
        .rsplit_once('-')
        .filter(|(_, suffix)| ["append", "prepend", "delete", "replace"].contains(suffix))
        .unwrap_or((command, "set"));
    global(interp, name);
    if interp.context::<State>(id).overrides.contains(name) {
        return Ok(Value::empty());
    }
    let mut values = if operation == "set" {
        Vec::new()
    } else {
        interp
            .scalar(name)
            .map(|value| value.as_list().map(|list| list.to_vec()))
            .unwrap_or(Ok(Vec::new()))?
    };
    match operation {
        "set" | "append" => values.extend_from_slice(&argv[1..]),
        "prepend" => {
            let mut args = argv[1..].to_vec();
            args.extend(values);
            values = args;
        }
        "delete" => {
            for value in &argv[1..] {
                if let Some(position) = values.iter().position(|v| v == value) {
                    values.remove(position);
                }
            }
        }
        "replace" => {
            for pair in argv[1..].chunks(2) {
                if let Some(position) = values.iter().position(|v| v == &pair[0]) {
                    values[position] = pair.get(1).cloned().unwrap_or_else(Value::empty);
                }
            }
        }
        _ => unreachable!(),
    }
    if name == "default_variants" && matches!(operation, "set" | "append") {
        super::variants::defaults(interp, id, &argv[1..])?;
    }

    let value = if matches!(
        name,
        "description" | "long_description" | "notes" | "homepage"
    ) {
        Value::from(
            values
                .iter()
                .map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" "),
        )
    } else {
        Value::from(values)
    };
    interp.set_scalar_return(name, value)
}
fn access(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 3, "name ?value?")?;
    let name = argv[1].as_str();
    global(interp, name);
    if let Some(value) = argv.get(2) {
        if name == "default_variants" {
            super::variants::defaults(interp, id, &value.as_list()?)?;
        }
        interp.set_scalar(name, value.clone())?;
    }
    interp.scalar(name)
}
fn exists(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "name")?;
    global(interp, argv[1].as_str());
    Ok(Value::from(interp.var_exists(&argv[1])))
}
fn hook(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "body")?;
    interp.context::<State>(id).hooks.push(Hook {
        phase: argv[0].to_string(),
        body: argv[1].to_string(),
    });
    Ok(Value::empty())
}
fn subport(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 3, "name body")?;
    global(interp, "subport");
    if interp.scalar("subport")? == argv[1] {
        interp.context::<State>(id).selected_subport = true;
        interp.eval_value(&argv[2])
    } else {
        Ok(Value::empty())
    }
}
