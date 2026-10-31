//! Native configure-log diagnostics, without grep or a Tcl runtime dependency.
use super::*;
use crate::progress::Progress;
use molt::{MoltResult, check_args};
use std::{fs, path::Path, sync::LazyLock};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    for (name, command) in [
        (
            "portconfigure::check_warnings",
            warnings as molt::CommandFunc,
        ),
        ("portconfigure::check_implicit_int", implicit_int),
        (
            "portconfigure::check_incompatible_function_pointer_types",
            incompatible,
        ),
        (
            "portconfigure::check_implicit_function_declarations",
            declarations,
        ),
        ("portconfigure::configure_finish", finish),
    ] {
        interp.add_context_command(name, command, id);
    }
}
fn enabled(interp: &mut Interp, id: ContextID, argv: &[Value]) -> Result<(), Exception> {
    if interp.context::<State>(id).system_executor.is_none() {
        commands::unsupported(interp, id, argv)?;
    }
    Ok(())
}
fn logs(interp: &mut Interp) -> Result<Vec<(String, Vec<u8>)>, Exception> {
    let work = interp.scalar("workpath")?;
    let root = Path::new(work.as_str());
    let mut logs = Vec::new();
    traverse::visit_paths(vec![root.into()], |path| {
        if matches!(
            path.file_name().and_then(|name| name.to_str()),
            Some("config.log" | "CMakeError.log" | "meson-log.txt")
        ) && path.is_file()
        {
            // Upstream catches grep failures (including unreadable log files).
            if let Ok(bytes) = fs::read(path) {
                logs.push((
                    path.strip_prefix(root)
                        .unwrap_or(path)
                        .to_string_lossy()
                        .into_owned(),
                    bytes,
                ));
            }
        }
        Ok(())
    })?;
    Ok(logs)
}
fn report(interp: &mut Interp, id: ContextID, flag: &str, details: Vec<String>) {
    if details.is_empty() {
        return;
    }
    let progress = interp
        .context::<State>(id)
        .progress
        .clone()
        .unwrap_or_else(|| Progress::new(false));
    progress.warning(&format!("Configuration logfiles contain indications of -W{flag}; check that features were not accidentally disabled:"));
    for detail in details {
        progress.message(&format!("  {detail}"));
    }
}
fn warnings(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "warning_flag")?;
    enabled(interp, id, argv)?;
    interp.with_global_scope(|interp| {
        let suffix = format!("-W{}]", argv[1]);
        let details = logs(interp)?
            .into_iter()
            .filter_map(|(path, bytes)| {
                bytes
                    .split(|byte| *byte == b'\n')
                    .any(|line| line.ends_with(suffix.as_bytes()))
                    .then(|| format!("found in {path}"))
            })
            .collect();
        report(interp, id, argv[1].as_str(), details);
        Ok(Value::empty())
    })
}
fn implicit_int(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    warnings(interp, id, &[argv[0].clone(), "implicit-int".into()])
}
fn incompatible(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    warnings(
        interp,
        id,
        &[
            argv[0].clone(),
            "incompatible-function-pointer-types".into(),
        ],
    )
}
fn declarations(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    enabled(interp, id, argv)?;
    interp.with_global_scope(|interp| {
        let whitelist = interp
            .scalar("configure.checks.implicit_function_declaration.whitelist")?
            .as_list()?
            .to_vec();
        let files = logs(interp)?;
        let found = undeclared(files, &whitelist);
        report(
            interp,
            id,
            "implicit-function-declaration",
            found
                .into_iter()
                .map(|(function, paths)| format!("{function}: found in {}", paths.join(", ")))
                .collect(),
        );
        Ok(Value::empty())
    })
}
fn undeclared(logs: Vec<(String, Vec<u8>)>, whitelist: &[Value]) -> BTreeMap<String, Vec<String>> {
    let whitelist: Vec<_> = whitelist
        .iter()
        .map(|pattern| crate::query::glob::Glob::new(pattern.as_str(), false))
        .collect();
    let mut found: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (path, bytes) in logs {
        for function in functions(&bytes) {
            if !whitelist.iter().any(|pattern| pattern.matches(&function)) {
                let files = found.entry(function).or_default();
                if !files.contains(&path) {
                    files.push(path.clone());
                }
            }
        }
    }
    found
}
fn functions(bytes: &[u8]) -> Vec<String> {
    // grep's binary-file diagnostic contains no matching compiler message.
    if bytes.contains(&0) {
        return Vec::new();
    }
    static FUNCTION: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
        r"(?:implicit declaration of function|implicitly declaring library function|call to undeclared function|call to undeclared library function) '([^']+)'"
    ).unwrap()
    });
    String::from_utf8_lossy(bytes)
        .split('\n')
        .filter(|line| line.ends_with("-Wimplicit-function-declaration]"))
        .filter_map(|line| FUNCTION.captures(line).map(|matches| matches[1].to_owned()))
        .collect()
}
fn finish(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    enabled(interp, id, argv)?;
    interp.with_global_scope(|interp| {
        if Path::new(interp.scalar("configure.dir")?.as_str()).is_dir() {
            for (option, command) in [
                (
                    "implicit_function_declaration",
                    "check_implicit_function_declarations",
                ),
                ("implicit_int", "check_implicit_int"),
                (
                    "incompatible_function_pointer_types",
                    "check_incompatible_function_pointer_types",
                ),
            ] {
                if interp
                    .scalar(&format!("configure.checks.{option}"))?
                    .as_bool()?
                {
                    interp.eval_value(&Value::from(vec![
                        format!("portconfigure::{command}").into(),
                    ]))?;
                }
            }
        }
        Ok(Value::empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn diagnostic(name: &str) -> Vec<u8> {
        format!("warning: call to undeclared function '{name}' [-Wimplicit-function-declaration]\n")
            .into_bytes()
    }
    #[test]
    fn recognizes_compiler_messages_only_with_exact_warning_suffix() {
        for wording in [
            "implicit declaration of function",
            "implicitly declaring library function",
            "call to undeclared function",
            "call to undeclared library function",
        ] {
            assert_eq!(
                functions(
                    format!("warning: {wording} 'foo' [-Wimplicit-function-declaration]\n")
                        .as_bytes()
                ),
                ["foo"]
            );
        }
        assert!(
            functions(
                b"call to undeclared function 'foo' [-Wimplicit-function-declaration] extra\n"
            )
            .is_empty()
        );
        assert!(
            functions(b"call to undeclared function 'foo' [-Wimplicit-function-declaration]\r\n")
                .is_empty()
        );
        let mut binary = diagnostic("foo");
        binary.push(0);
        assert!(functions(&binary).is_empty());
    }
    #[test]
    fn whitelist_globs_are_case_insensitive_and_findings_deduplicate_paths() {
        let logs = vec![
            (
                "config.log".into(),
                [
                    diagnostic("GetThing"),
                    diagnostic("missing"),
                    diagnostic("missing"),
                ]
                .concat(),
            ),
            ("sub/meson-log.txt".into(), diagnostic("missing")),
        ];
        let found = undeclared(logs, &["get*".into()]);
        assert_eq!(found.len(), 1);
        assert_eq!(found["missing"], ["config.log", "sub/meson-log.txt"]);
    }
    #[test]
    fn log_walk_selects_nested_logs_without_descending_directory_links() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("config.log"), "outside").unwrap();
        fs::create_dir(root.path().join("nested")).unwrap();
        fs::write(root.path().join("nested/CMakeError.log"), "nested").unwrap();
        fs::write(root.path().join("ordinary.txt"), "ignored").unwrap();
        symlink(outside.path(), root.path().join("linked-directory")).unwrap();
        symlink(
            outside.path().join("config.log"),
            root.path().join("config.log"),
        )
        .unwrap();
        let mut interp = Interp::new();
        interp
            .set_scalar("workpath", root.path().to_str().unwrap().into())
            .unwrap();
        let mut files = logs(&mut interp).unwrap();
        files.sort();
        assert_eq!(
            files,
            vec![
                ("config.log".into(), b"outside".to_vec()),
                ("nested/CMakeError.log".into(), b"nested".to_vec())
            ]
        );
    }
}
