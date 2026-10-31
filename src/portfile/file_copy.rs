//! Tcl `file copy` for execution-enabled Portfile hooks.
use super::*;
use molt::MoltResult;
use std::{
    fs, io,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};

pub(super) fn command(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        interp
            .context::<State>(id)
            .unsupported
            .insert("file copy".into());
        return Err(error("unsupported Portfile command file copy"));
    }
    let mut start = 2;
    let mut force = false;
    while let Some(arg) = argv.get(start) {
        match arg.as_str() {
            "-force" => force = true,
            "--" => {
                start += 1;
                break;
            }
            option if option.starts_with('-') => {
                return Err(error(format!(
                    "bad option \"{option}\": must be -force or --"
                )));
            }
            _ => break,
        }
        start += 1;
    }
    let names: Vec<&str> = argv[start..].iter().map(Value::as_str).collect();
    let [sources @ .., target] = names.as_slice() else {
        return Err(error(
            "wrong # args: should be \"file copy ?-option value ...? source ?source ...? target\"",
        ));
    };
    if sources.is_empty() {
        return Err(error(
            "wrong # args: should be \"file copy ?-option value ...? source ?source ...? target\"",
        ));
    }
    let target = PathBuf::from(target);
    let into = target.is_dir();
    if sources.len() > 1 && !into {
        return Err(error(format!(
            "error copying: target \"{}\" is not a directory",
            target.display()
        )));
    }
    for source in sources {
        let from = Path::new(source);
        if fs::symlink_metadata(from).is_err() {
            return Err(error(format!(
                "error copying \"{source}\": no such file or directory"
            )));
        }
        let to = if into {
            target.join(from.file_name().unwrap_or_default())
        } else {
            target.clone()
        };
        if fs::symlink_metadata(&to).is_ok() {
            if !force {
                return Err(error(format!(
                    "error copying \"{source}\" to \"{}\": file already exists",
                    to.display()
                )));
            }
            if to.is_dir() && !fs::symlink_metadata(&to).is_ok_and(|m| m.file_type().is_symlink()) {
                if !from.is_dir() {
                    return Err(error(format!(
                        "error copying \"{source}\" to \"{}\": file already exists",
                        to.display()
                    )));
                }
            } else {
                fs::remove_file(&to).map_err(|err| failure_message(source, &to, err))?;
            }
        }
        copy_tree(from, &to).map_err(|err| failure_message(source, &to, err))?;
    }
    Ok(Value::empty())
}

fn failure_message(source: &str, to: &Path, err: io::Error) -> Exception {
    error(format!(
        "error copying \"{source}\" to \"{}\": {err}",
        to.display()
    ))
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(from)?;
    if metadata.file_type().is_symlink() {
        symlink(fs::read_link(from)?, to)?;
    } else if metadata.is_dir() {
        if !to.is_dir() {
            fs::create_dir(to)?;
        }
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        fs::set_permissions(to, metadata.permissions())?;
    } else {
        fs::copy(from, to)?;
        if let Ok(modified) = metadata.modified()
            && let Ok(handle) = fs::File::options().write(true).open(to)
        {
            let _ = handle.set_modified(modified);
        }
    }
    Ok(())
}
