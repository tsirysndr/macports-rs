//! XML generation for services launched directly through daemondo.
use super::*;

pub(super) struct Generated {
    pub xml: String,
    pub wrapper: Option<(String, String)>,
    pub keep_run: bool,
    pub warnings: Vec<String>,
}
fn flag(interp: &mut Interp, attrs: &Attributes, key: &str) -> Result<bool> {
    attribute(interp, attrs, key)?
        .map(|v| v.as_bool().map_err(failure))
        .transpose()
        .map(|v| v.unwrap_or(false))
}
fn escaped(text: &str) -> Result<String> {
    let mut result = String::new();
    for c in text.chars() {
        ensure!(
            matches!(c, '\t' | '\n' | '\r')
                || ('\u{20}'..='\u{d7ff}').contains(&c)
                || ('\u{e000}'..='\u{fffd}').contains(&c)
                || c >= '\u{10000}',
            "invalid XML character in startup item"
        );
        match c {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&apos;"),
            _ => result.push(c),
        }
    }
    Ok(result)
}
fn string(xml: &mut String, key: &str, value: &str) -> Result<()> {
    xml.push_str(&format!(
        "<key>{key}</key><string>{}</string>\n",
        escaped(value)?
    ));
    Ok(())
}
pub(super) fn generate(
    interp: &mut Interp,
    attrs: &Attributes,
    name: &str,
    unique: &str,
    location: &str,
) -> Result<Generated> {
    let prefix = interp.scalar("prefix").map_err(failure)?.to_string();
    let executable = attr_text(interp, attrs, "executable", "")?;
    let wrapper = wrapper::generate(interp, attrs, name, &prefix, &executable)?;
    let mut args = vec![format!("{prefix}/bin/daemondo"), format!("--label={name}")];
    if let Some((filename, _)) = &wrapper {
        let path = format!("{prefix}/etc/{location}/{unique}/{filename}");
        for phase in ["start", "stop", "restart"] {
            args.extend([
                format!("--{phase}-cmd"),
                path.clone(),
                phase.into(),
                ";".into(),
            ]);
        }
    } else {
        args.push("--start-cmd".into());
        args.extend(
            Value::from(executable.clone())
                .as_list()
                .map_err(failure)?
                .iter()
                .map(ToString::to_string),
        );
        args.push(";".into());
    }
    if flag(interp, attrs, "netchange")? {
        args.push("--restart-netchange".into());
    }
    if flag(interp, attrs, "logevents")? {
        // Upstream explicitly reads the global verbosity option, even for a
        // multi-item definition that supplies an item-local value.
        args.push(format!(
            "--verbosity={}",
            text(interp, "startupitem.daemondo.verbosity", "1")?
        ));
    }
    let pid = Value::from(attr_text(interp, attrs, "pidfile", "")?)
        .as_list()
        .map_err(failure)?
        .to_vec();
    let keep_run = pid.len() == 1;
    let mut warnings = Vec::new();
    if pid.is_empty() {
        args.push(
            if executable.is_empty() {
                "--pid=none"
            } else {
                "--pid=exec"
            }
            .into(),
        );
    } else {
        if pid.len() > 2 {
            warnings.push(format!(
                "Invalid parameter count to startupitem.pidfile: 2 expected, {} found",
                pid.len()
            ));
        }
        let file = pid.get(1).map_or_else(
            || format!("{prefix}/var/run/{name}.pid"),
            ToString::to_string,
        );
        match pid[0].as_str() {
            "none" => args.push("--pid=none".into()),
            "auto" | "clean" | "manual" => {
                args.push(
                    match pid[0].as_str() {
                        "auto" => "--pid=fileauto",
                        "clean" => "--pid=fileclean",
                        _ => "--pid=exec",
                    }
                    .into(),
                );
                args.extend(["--pidfile".into(), file]);
            }
            other => warnings.push(format!(
                "Unknown pidfile style {other} presented to startupitem.pidfile"
            )),
        }
    }
    let mut xml = "<?xml version='1.0' encoding='UTF-8'?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\"\n\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\" >\n<plist version='1.0'>\n<dict>\n".to_owned();
    string(&mut xml, "Label", unique)?;
    xml.push_str("<key>ProgramArguments</key>\n<array>\n");
    for arg in args {
        xml.push_str(&format!("\t<string>{}</string>\n", escaped(&arg)?));
    }
    xml.push_str("</array>\n<key>Disabled</key><true/>\n<key>KeepAlive</key><true/>\n");
    for (key, option) in [
        ("UserName", "user"),
        ("GroupName", "group"),
        ("StandardOutPath", "logfile"),
    ] {
        let value = attr_text(interp, attrs, option, "")?;
        if !value.is_empty() {
            string(&mut xml, key, &value)?;
        }
    }
    // The upstream stderr default references the global stdout option.
    let stdout = text(interp, "startupitem.logfile", "")?;
    let stderr = attr_text(interp, attrs, "logfile.stderr", &stdout)?;
    if !stderr.is_empty() {
        string(&mut xml, "StandardErrorPath", &stderr)?;
    }
    if flag(interp, attrs, "debug")? {
        xml.push_str("<key>Debug</key><true/>\n");
    }
    xml.push_str("</dict>\n</plist>\n");
    Ok(Generated {
        xml,
        wrapper,
        keep_run,
        warnings,
    })
}
