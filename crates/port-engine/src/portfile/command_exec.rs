//! PortSystem command strings, environment helpers, and explicitly enabled execution.
use super::*;
use molt::{MoltResult, check_args};
use std::path::PathBuf;

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    for name in [
        "command_string",
        "parse_environment",
        "append_to_environment_value",
        "environment_array_to_string",
        "command_exec",
    ] {
        interp.add_context_command(name, command, id);
    }
    for name in [
        "compiler.cpath",
        "compiler.library_path",
        "compiler.log_verbose_output",
        "source_date_epoch",
    ] {
        commands::register(interp, id, name);
    }
}
fn optional(interp: &mut Interp, name: &str) -> Result<Option<Value>, Exception> {
    if interp.var_exists(&name.into()) {
        interp.scalar(name).map(Some)
    } else {
        Ok(None)
    }
}
fn string(interp: &mut Interp, name: &str) -> MoltResult {
    let mut result = String::new();
    if let Some(directory) = optional(interp, &format!("{name}.dir"))? {
        result.push_str(&format!("cd \"{directory}\" &&"));
    }
    let cmd = optional(interp, &format!("{name}.cmd"))?
        .unwrap_or_else(|| Value::from(vec![Value::from(name)]));
    for value in cmd.as_list()?.iter() {
        result.push(' ');
        result.push_str(value.as_str());
    }
    for suffix in ["pre_args", "args", "post_args"] {
        if let Some(values) = optional(interp, &format!("{name}.{suffix}"))? {
            for value in values.as_list()?.iter() {
                result.push(' ');
                result.push_str(value.as_str());
            }
        }
    }
    Ok(result.into())
}
fn parse_environment(interp: &mut Interp, name: &str) -> MoltResult {
    let array = format!("{name}.env_array");
    if let Some(values) = optional(interp, &format!("{name}.env"))? {
        for value in values.as_list()?.iter() {
            if let Some((key, value)) = value.as_str().split_once('=') {
                interp.set_element(&array, key, value.into())?;
            }
        }
    } else {
        interp.array_set(&array, &[])?;
    }
    Ok(Value::empty())
}
fn command(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    match argv[0].as_str() {
        "command_string" => {
            check_args(1, argv, 2, 2, "command")?;
            interp.with_global_scope(|interp| string(interp, argv[1].as_str()))
        }
        "parse_environment" => {
            check_args(1, argv, 2, 2, "command")?;
            interp.with_global_scope(|interp| parse_environment(interp, argv[1].as_str()))
        }
        "append_to_environment_value" => {
            check_args(1, argv, 3, 0, "command key ?value ...?")?;
            interp.with_global_scope(|interp| {
                let array = format!("{}.env_array", argv[1]);
                let key = argv[2].as_str();
                let mut value = interp.element(&array, key).ok().map(|v| v.to_string());
                for next in &argv[3..] {
                    if !next.as_str().is_empty() {
                        let value = value.get_or_insert_with(String::new);
                        value.push(' ');
                        value.push_str(next.as_str());
                    }
                }
                if let Some(value) = value {
                    interp.set_element(&array, key, value.trim_start().into())?;
                    return Ok(0.into());
                }
                Ok(1.into())
            })
        }
        "environment_array_to_string" => {
            check_args(1, argv, 2, 2, "environment_array")?;
            // Unlike command option globals, this helper reads the caller's scope.
            let values = interp.array_get(argv[1].as_str());
            let mut lines = values
                .as_chunks::<2>()
                .0
                .iter()
                .map(|[key, value]| format!("{key}='{value}'"))
                .collect::<Vec<_>>();
            lines.sort();
            Ok(format!("\n{}", lines.join("\n")).into())
        }
        "command_exec" => {
            if interp.context::<State>(id).system_executor.is_none() {
                return commands::unsupported(interp, id, argv);
            }
            interp.with_global_scope(|interp| execute(interp, id, argv))
        }
        _ => unreachable!(),
    }
}
fn execute(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    let mut prefix = None;
    let mut new_session = false;
    let mut cursor = 1;
    while cursor < argv.len() && argv[cursor].as_str().starts_with('-') {
        match argv[cursor].as_str() {
            "-notty" => new_session = true,
            "--" => {
                cursor += 1;
                break;
            }
            "-varprefix" => {
                cursor += 1;
                if cursor >= argv.len() {
                    return Err(error("-varprefix requires a value"));
                }
                prefix = Some(argv[cursor].to_string());
            }
            flag => {
                interp
                    .context::<State>(id)
                    .unsupported
                    .insert(format!("command_exec {flag}"));
                return Err(error(format!("unsupported command_exec option {flag}")));
            }
        }
        cursor += 1;
    }
    if cursor == argv.len() {
        return Err(error("Missing command argument"));
    }
    if argv.len() - cursor > 3 {
        return Err(error("too many command_exec arguments"));
    }
    let name = argv[cursor].as_str();
    let prefix = prefix
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| name.into());
    let directory = interp.scalar(&format!("{prefix}.dir"))?;
    if directory.as_str().is_empty() {
        return Err(error("command directory must not be empty"));
    }
    let nice = optional(interp, &format!("{prefix}.nice"))?
        .filter(|value| !value.as_str().is_empty())
        .map(|value| system::parse_nice(&value))
        .transpose()?
        .flatten();
    let array = format!("{prefix}.env_array");
    // Through the interpreter, as upstream: PortGroups such as muniversal
    // rename and wrap parse_environment and command_string.
    if !interp.array_exists(&array) {
        interp.eval_value(&Value::from(vec![
            Value::from("parse_environment"),
            Value::from(prefix.as_str()),
        ]))?;
    }
    // These options come from initialized PortSystem configuration. Do not
    // silently invent compiler/SDK settings when the host has not supplied them.
    for (option, key) in [
        ("macosx_deployment_target", "MACOSX_DEPLOYMENT_TARGET"),
        ("configure.sdkroot", "SDKROOT"),
        ("source_date_epoch", "SOURCE_DATE_EPOCH"),
    ] {
        let value = interp.scalar(option)?;
        if !value.as_str().is_empty() {
            interp.set_element(&array, key, value)?;
        }
    }
    if interp.scalar("compiler.log_verbose_output")?.as_bool()? {
        let workpath = interp.scalar("workpath")?;
        interp.set_element(&array, "CC_PRINT_OPTIONS", "YES".into())?;
        interp.set_element(
            &array,
            "CC_PRINT_OPTIONS_FILE",
            PathBuf::from(workpath.as_str())
                .join(".CC_PRINT_OPTIONS")
                .to_string_lossy()
                .into_owned()
                .into(),
        )?;
    }
    for (option, key) in [
        ("compiler.cpath", "CPATH"),
        ("compiler.library_path", "LIBRARY_PATH"),
    ] {
        let value = interp.scalar(option)?;
        if !value.as_str().is_empty() {
            interp.set_element(
                &array,
                key,
                value
                    .as_list()?
                    .iter()
                    .map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(":")
                    .into(),
            )?;
        }
    }
    let developer = interp.scalar("configure.developer_dir")?;
    interp.set_element(&array, "DEVELOPER_DIR", developer)?;
    let script = interp.eval_value(&Value::from(vec![
        Value::from("command_string"),
        Value::from(name),
    ]))?;
    let before = argv.get(cursor + 1).map(Value::as_str).unwrap_or("");
    let after = argv.get(cursor + 2).map(Value::as_str).unwrap_or("");
    let saved = interp.array_get("env");
    let overrides = interp.array_get(&array);
    interp.array_set("env", &overrides)?;
    let environment = interp
        .array_get("env")
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[key, value]| (key.to_string(), value.to_string()))
        .collect();
    let creation_mask = interp.context::<State>(id).creation_mask;
    let result = system::execute_request(
        interp,
        id,
        SystemRequest {
            script: format!("{before} {script} {after}"),
            directory: None,
            prepare_directory: Some(directory.as_str().into()),
            creation_mask,
            nice,
            new_session,
            environment,
        },
    );
    interp.unset(&array);
    interp.unset("env");
    interp.array_set("env", &saved)?;
    result
}
