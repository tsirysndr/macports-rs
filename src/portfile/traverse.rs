//! Native physical tree traversal: no process cwd changes or external tools.
use super::*;
use molt::{MoltResult, ResultCode, check_args};
use std::{
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("fs-traverse", command, id);
    interp.add_context_command("portutil::get_latest_path_mtime", latest_mtime, id);
}
#[derive(Default, Clone, Copy)]
struct Options {
    depth: bool,
    ignore_errors: bool,
    tails: bool,
}
enum Action {
    Next,
    Skip,
    Stop,
}
struct Entry {
    path: PathBuf,
    root: bool,
    device: Option<u64>,
    post: bool,
}

fn walk(
    paths: Vec<PathBuf>,
    options: Options,
    mut visit: impl FnMut(&Path) -> Result<Action, Exception>,
) -> Result<(), Exception> {
    let mut paths = paths;
    paths.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    let mut pending: Vec<_> = paths
        .into_iter()
        .rev()
        .map(|path| Entry {
            path,
            root: true,
            device: None,
            post: false,
        })
        .collect();
    while let Some(entry) = pending.pop() {
        if entry.post {
            if matches!(visit(&entry.path)?, Action::Stop) {
                break;
            }
            continue;
        }
        let metadata = if entry.root {
            fs::metadata(&entry.path).or_else(|_| fs::symlink_metadata(&entry.path))
        } else {
            fs::symlink_metadata(&entry.path)
        };
        let metadata = match metadata {
            Ok(metadata) => metadata,
            Err(_) if options.ignore_errors => continue,
            Err(err) => return Err(traversal_error(&entry.path, err)),
        };
        let directory = metadata.is_dir();
        if !directory || !options.depth {
            match visit(&entry.path)? {
                Action::Stop => break,
                Action::Skip => continue,
                Action::Next => (),
            }
        }
        if !directory {
            continue;
        }
        let device = entry.device.unwrap_or(metadata.dev());
        // Like FTS_XDEV, visit mount points but do not descend across devices.
        if options.depth {
            pending.push(Entry {
                path: entry.path.clone(),
                root: false,
                device: Some(device),
                post: true,
            });
        }
        if metadata.dev() != device {
            continue;
        }
        let children = match fs::read_dir(&entry.path) {
            Ok(children) => children,
            Err(_) if options.ignore_errors => continue,
            Err(err) => return Err(traversal_error(&entry.path, err)),
        };
        let mut paths = Vec::new();
        for child in children {
            match child {
                Ok(child) => paths.push(child.path()),
                Err(_) if options.ignore_errors => (),
                Err(err) => return Err(traversal_error(&entry.path, err)),
            }
        }
        paths.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
        pending.extend(paths.into_iter().rev().map(|path| Entry {
            path,
            root: false,
            device: Some(device),
            post: false,
        }));
    }
    Ok(())
}

fn traversal_error(path: &Path, err: io::Error) -> Exception {
    let exception = filesystem::file_error(&path.to_string_lossy(), false, err);
    let code = exception.error_code();
    let words = code.as_list().expect("POSIX error is a list");
    Exception::molt_err2(
        code.clone(),
        format!("{}: {}", path.display(), words[2]).into(),
    )
}

fn command(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    let mut options = Options::default();
    let mut start = 1;
    while let Some(value) = argv.get(start) {
        match value.as_str() {
            "-depth" => options.depth = true,
            "-ignoreErrors" => options.ignore_errors = true,
            "-tails" => options.tails = true,
            "--" => {
                start += 1;
                break;
            }
            _ => break,
        }
        start += 1;
    }
    check_args(
        1,
        argv,
        start + 3,
        start + 3,
        "?-depth? ?-ignoreErrors? ?-tails? ?--? varname target-list body",
    )?;
    let paths = argv[start + 1].as_list()?;
    if options.tails && paths.len() > 1 {
        return Err(error("-tails cannot be used with multiple paths"));
    }
    let paths: Vec<PathBuf> = paths
        .iter()
        .map(|path| PathBuf::from(path.as_str()))
        .collect();
    let root = paths.first().cloned().unwrap_or_default();
    let mut result = Value::empty();
    walk(paths, options, |path| {
        let path = if options.tails {
            path.strip_prefix(&root).unwrap_or(path)
        } else {
            path
        };
        let text = if options.tails && path.as_os_str().is_empty() {
            "."
        } else {
            path.to_str()
                .ok_or_else(|| error("fs-traverse path is not valid UTF-8"))?
        };
        let assigned = interp.set_var(&argv[start], text.into());
        if !options.ignore_errors {
            assigned?;
        }
        match interp.eval_value(&argv[start + 2]) {
            Ok(value) => {
                result = value;
                Ok(Action::Next)
            }
            Err(err) if err.code() == ResultCode::Continue => {
                result = err.value();
                Ok(Action::Skip)
            }
            Err(err) if err.code() == ResultCode::Break => {
                result = err.value();
                Ok(Action::Stop)
            }
            Err(err) => Err(err),
        }
    })?;
    Ok(result)
}

fn latest_mtime(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "checkpaths")?;
    let paths = argv[1]
        .as_list()?
        .iter()
        .map(|path| PathBuf::from(path.as_str()))
        .collect();
    let mut newest = 0;
    walk(paths, Options::default(), |path| {
        // The PortSystem helper ignores errors in its body and excludes links
        // and directories, even when a root symlink is traversed as a directory.
        if let Ok(metadata) = fs::symlink_metadata(path)
            && metadata.is_file()
        {
            newest = newest.max(metadata.mtime());
        }
        Ok(Action::Next)
    })?;
    Ok(newest.into())
}
