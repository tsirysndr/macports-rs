//! Canonical PortSystem architectures and staged binary diagnostics.
use super::*;
use molt::{MoltResult, check_args};
use std::{
    fs,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("fileIsBinary", binary, id);
    interp.add_context_command("get_canonical_archs", canonical, id);
    interp.add_context_command("get_canonical_archflags", flags, id);
    interp.add_context_command("porttest::test_archs", check, id);
}
fn binary(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "filename")?;
    let path = argv[1].as_str();
    let metadata =
        fs::symlink_metadata(path).map_err(|err| filesystem::file_error(path, false, err))?;
    if !metadata.is_file() {
        return Ok(false.into());
    }
    // Do not follow a leaf replaced between lstat and open, or block if it
    // becomes a FIFO. Parent aliases follow normal Portfile filesystem rules.
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|err| filesystem::file_error(path, false, err))?;
    if !file
        .metadata()
        .map_err(|err| filesystem::file_error(path, false, err))?
        .is_file()
    {
        return Ok(false.into());
    }
    crate::macho::is_binary(&mut file)
        .map(Value::from)
        .map_err(|err| match err.downcast::<std::io::Error>() {
            Ok(err) => filesystem::file_error(path, false, err),
            Err(err) => error(format!("cannot classify {path}: {err}")),
        })
}
fn canonical(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| {
        if interp.scalar("supported_archs")?.as_str() == "noarch" {
            return Ok("noarch".into());
        }
        let universal = universal_enabled(interp)?;
        if universal {
            let value = interp.scalar("configure.universal_archs")?;
            let archs = value.as_list()?;
            if archs.len() >= 2 {
                let mut archs = archs.iter().map(ToString::to_string).collect::<Vec<_>>();
                archs.sort();
                return Ok(list(archs));
            }
        }
        let arch = interp.scalar("configure.build_arch")?;
        if !arch.as_str().is_empty() {
            Ok(arch)
        } else {
            interp.scalar("os.arch")
        }
    })
}

fn check(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| {
        if interp.scalar("os.platform")?.as_str() != "darwin"
            || interp.scalar("test.ignore_archs")?.as_bool()?
        {
            return Ok(Value::empty());
        }
        // Root-only files in the destroot need root to read, as upstream.
        let destroot = interp.scalar("destroot")?;
        let destpath = interp.scalar("destpath")?;
        let user = interp.scalar("macports::macportsuser")?;
        let (files, warnings) = crate::privileges::as_root(user.as_str(), || {
            inspect(Path::new(destpath.as_str()), destroot.as_str())
        })?;
        for warning in warnings {
            eprintln!("Warning: {warning}");
        }
        let wanted = interp.eval_value(&Value::from(vec!["get_canonical_archs".into()]))?;
        let wanted = wanted
            .as_list()?
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let subport = interp.scalar("subport")?;
        if let Some(warning) = mismatch_warning(subport.as_str(), &wanted, files) {
            eprintln!("Warning: {warning}");
        }
        Ok(Value::empty())
    })
}

type Files = BTreeMap<Vec<String>, Vec<String>>;
fn inspect(root: &Path, destroot: &str) -> Result<(Files, Vec<String>), Exception> {
    let mut files = BTreeMap::new();
    let mut warnings = Vec::new();
    traverse::visit_paths(vec![PathBuf::from(root)], |path| {
        let metadata = fs::symlink_metadata(path)
            .map_err(|err| filesystem::file_error(&path.to_string_lossy(), false, err))?;
        if !metadata.is_file() {
            return Ok(());
        }
        let text = path
            .to_str()
            .ok_or_else(|| error("staged path is not valid UTF-8"))?;
        if !filesystem::predicate("readable", text) {
            return Ok(());
        }
        let architectures = fs::File::open(path)
            .map_err(anyhow::Error::from)
            .and_then(|mut file| crate::macho::architectures(&mut file));
        match architectures {
            Ok(architectures) if !architectures.is_empty() => {
                let suffix = text.strip_prefix(destroot).unwrap_or(text);
                files
                    .entry(architectures.into_iter().collect::<Vec<_>>())
                    .or_insert_with(Vec::new)
                    .push(suffix.to_owned());
            }
            Ok(_) => (),
            Err(err) => warnings.push(format!("Error parsing file {text}: {err}")),
        }
        Ok(())
    })?;
    Ok((files, warnings))
}

fn mismatch_warning(subport: &str, wanted: &[String], mut files: Files) -> Option<String> {
    let has_wanted = files.remove(wanted).is_some();
    let noarch = wanted == ["noarch"];
    if !files.is_empty() {
        let target = if noarch {
            "no architecture-specific files,".to_owned()
        } else {
            format!("for the architecture(s) '{}',", wanted.join(" "))
        };
        let mut message = format!(
            "{subport} is configured to build {target} but installed Mach-O files built for the following archs:\n"
        );
        for (archs, paths) in files {
            message.push_str(&format!("{}:\n", archs.join(",")));
            for path in paths {
                message.push_str(&format!("  {path}\n"));
            }
        }
        Some(message)
    } else if !noarch && !has_wanted {
        Some(format!(
            "{subport} is configured to build for the architecture(s) '{}', but did not install any Mach-O files.",
            wanted.join(" ")
        ))
    } else {
        None
    }
}

pub(super) fn universal_enabled(interp: &mut Interp) -> Result<bool, Exception> {
    for command in ["variant_exists", "variant_isset"] {
        if !interp
            .eval_value(&Value::from(vec![command.into(), "universal".into()]))?
            .as_bool()?
        {
            return Ok(false);
        }
    }
    Ok(true)
}
fn flags(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 2, "?tool?")?;
    interp.with_global_scope(|interp| {
        let tool = argv.get(1).map(Value::as_str).unwrap_or("cc");
        let universal = universal_enabled(interp)?
            && interp.scalar("configure.universal_archs")?.as_list()?.len() >= 2;
        let (option, description) = if universal {
            let tool = if tool == "cc" { "c" } else { tool };
            (
                format!("configure.universal_{tool}flags"),
                format!("universal archflags do not exist for tool '{tool}'"),
            )
        } else {
            (
                format!("configure.{tool}_archflags"),
                format!("archflags do not exist for tool '{tool}'"),
            )
        };
        interp.scalar(&option).map_err(|_| error(description))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    fn object(cpu: u32) -> Vec<u8> {
        [0xfeedfacf, cpu, 0, 1, 0, 0, 0, 0]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect()
    }
    #[test]
    fn staged_scan_groups_binary_architectures_and_ignores_links_and_text() {
        let tree = tempfile::tempdir().unwrap();
        fs::write(tree.path().join("intel"), object(0x1000007)).unwrap();
        fs::write(tree.path().join("arm"), object(0x100000c)).unwrap();
        fs::write(tree.path().join("readme"), "plain text").unwrap();
        fs::write(tree.path().join("broken"), [0xcf, 0xfa, 0xed, 0xfe]).unwrap();
        symlink("intel", tree.path().join("link")).unwrap();
        let (files, warnings) = inspect(tree.path(), tree.path().to_str().unwrap()).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(warnings.len(), 1);
        let warning = mismatch_warning("app", &["arm64".into()], files.clone()).unwrap();
        assert!(warning.contains("x86_64:\n  /intel\n"));
        assert!(!warning.contains("/arm"));
        assert!(!warning.contains("/link"));
        assert!(
            mismatch_warning("app", &["noarch".into()], files)
                .unwrap()
                .contains("no architecture-specific files")
        );
    }
    #[test]
    fn matching_binaries_and_noarch_data_do_not_warn_but_missing_binaries_do() {
        let mut files = Files::new();
        files.insert(vec!["arm64".into()], vec!["/binary".into()]);
        assert!(mismatch_warning("app", &["arm64".into()], files).is_none());
        assert!(mismatch_warning("app", &["noarch".into()], Files::new()).is_none());
        assert!(
            mismatch_warning("app", &["arm64".into()], Files::new())
                .unwrap()
                .contains("did not install any Mach-O files")
        );
    }
}
