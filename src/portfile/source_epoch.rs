//! `portmain::get_source_date_epoch`: reproducible-build timestamp of the port.
use super::*;
use molt::{MoltResult, check_args};
use std::{path::Path, process::Command};

pub(super) fn install(interp: &mut Interp, id: ContextID) -> MoltResult {
    interp.add_context_command("portmain::get_source_date_epoch", epoch, id);
    commands::register(interp, id, "source_date_epoch");
    if !interp.var_exists(&"source_date_epoch".into()) {
        defaults::register(
            interp,
            id,
            "source_date_epoch",
            "[portmain::get_source_date_epoch]",
        )?;
        interp
            .context::<State>(id)
            .execution_defaults
            .insert("source_date_epoch".into());
    }
    Ok(Value::empty())
}

fn git(directory: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("/usr/bin/git")
        .args(["-c", "safe.directory=*", "-C"])
        .arg(directory)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn newest_mtime(path: &Path, newest: &mut i64) {
    use std::os::unix::fs::MetadataExt;
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return;
    };
    if metadata.is_dir() {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                newest_mtime(&entry.path(), newest);
            }
        }
    } else if metadata.is_file() {
        *newest = (*newest).max(metadata.mtime());
    }
}

fn epoch(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    if let Some(cached) = interp.context::<State>(id).source_date_epoch {
        return Ok(cached.into());
    }
    let (portpath, filespath) = interp.with_global_scope(|interp| {
        Ok::<_, Exception>((
            interp.scalar("portpath")?.to_string(),
            interp.scalar("filespath")?.to_string(),
        ))
    })?;
    let groups: Vec<std::path::PathBuf> = interp
        .context::<State>(id)
        .loaded_groups
        .iter()
        .filter_map(|group| group.path.clone())
        .collect();
    let portdir = Path::new(&portpath);
    let mut checks: Vec<(std::path::PathBuf, std::path::PathBuf)> =
        vec![(portdir.to_path_buf(), portdir.to_path_buf())];
    for group in &groups {
        checks.push((group.clone(), group.parent().unwrap_or(portdir).to_path_buf()));
    }
    let in_git = Path::new("/usr/bin/git").exists()
        && checks.iter().all(|(_, dir)| {
            git(dir, &["rev-parse", "--is-inside-work-tree"]).is_some_and(|out| out == "true")
        });
    let mut newest = 0i64;
    let mut resolved = false;
    if in_git {
        let clean = checks.iter().all(|(path, dir)| {
            git(dir, &["status", "--porcelain", &path.to_string_lossy()]).is_some_and(|out| out.is_empty())
        });
        if clean {
            resolved = true;
            for (path, dir) in &checks {
                match git(dir, &["log", "-1", "--pretty=%ct", &path.to_string_lossy()])
                    .and_then(|out| out.parse::<i64>().ok())
                {
                    Some(time) => newest = newest.max(time),
                    None => {
                        resolved = false;
                        break;
                    }
                }
            }
        }
    }
    if !resolved {
        newest = 0;
        newest_mtime(&portdir.join("Portfile"), &mut newest);
        if Path::new(&filespath).is_dir() {
            newest_mtime(Path::new(&filespath), &mut newest);
        }
        for group in &groups {
            newest_mtime(group, &mut newest);
        }
    }
    interp.context::<State>(id).source_date_epoch = Some(newest);
    Ok(newest.into())
}
