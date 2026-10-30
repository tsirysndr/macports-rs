//! Generate the shell phase dispatcher used by MacPorts' daemondo services.
use super::*;

fn lines(interp: &mut Interp, attrs: &Attributes, key: &str) -> Result<Vec<String>> {
    Ok(Value::from(attr_text(interp, attrs, key, "")?)
        .as_list()
        .map_err(failure)?
        .iter()
        .map(ToString::to_string)
        .collect())
}
fn quoted(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/_-.".contains(&b))
    {
        value.into()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}
pub(super) fn generate(
    interp: &mut Interp,
    attrs: &Attributes,
    name: &str,
    prefix: &str,
    executable: &str,
) -> Result<Option<(String, String)>> {
    let init = lines(interp, attrs, "init")?;
    let mut start = lines(interp, attrs, "start")?;
    let mut stop = lines(interp, attrs, "stop")?;
    let mut restart = lines(interp, attrs, "restart")?;
    if !executable.is_empty()
        && [&init, &start, &stop, &restart]
            .iter()
            .all(|lines| lines.is_empty())
    {
        return Ok(None);
    }
    component(name)?;
    let subport = interp.scalar("subport").map_err(failure)?.to_string();
    let script = quoted(&format!("{prefix}/etc/startup/{subport}.sh"));
    if start.is_empty() {
        start.push(format!("sh {script} start"));
    }
    if stop.is_empty() {
        stop.push(format!("sh {script} stop"));
    }
    if restart.is_empty() {
        restart.extend(["Stop".into(), "Start".into()]);
    }
    let mut output = format!(
        "#!/bin/sh\n#\n# MacPorts generated daemondo support script\n#\n\n#\n# Init\n#\nprefix={}\n",
        quoted(prefix)
    );
    for line in init {
        output.push_str(&line);
        output.push('\n');
    }
    output.push('\n');
    for (label, lines) in [("Start", start), ("Stop", stop), ("Restart", restart)] {
        output.push_str(&format!("#\n# {label}\n#\n{label}()\n{{\n"));
        for line in lines {
            output.push('\t');
            output.push_str(&line);
            output.push('\n');
        }
        output.push_str("}\n\n");
    }
    output.push_str("#\n# Run\n#\nRun()\n{\ncase $1 in\n  start  ) Start   ;;\n  stop   ) Stop    ;;\n  restart) Restart ;;\n  *      ) echo \"$0: unknown argument: $1\";;\nesac\n}\n\n#\n# Run a phase based on the selector\n#\nRun $1\n\n");
    ensure!(!output.contains('\0'), "NUL in startup wrapper script");
    Ok(Some((format!("{name}.wrapper"), output)))
}
