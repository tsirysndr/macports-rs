//! Lexical Unix paths following MacPorts' bundled Tcl 9 implementation.
//! Keep dots, parent components, and tildes literal. Read-only probes are
//! dispatched separately so lexical operations never inspect the filesystem.
use super::*;
use molt::{MoltResult, check_args};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("file", file, id);
}

fn root_len(path: &str) -> usize {
    if path.starts_with("//") && path.len() > 2 && !path.starts_with("///") {
        2 + path[2..].find('/').unwrap_or(path.len() - 2)
    } else {
        usize::from(path.starts_with('/'))
    }
}

pub(super) fn split(path: &str) -> Vec<&str> {
    let root = root_len(path);
    let mut parts = Vec::new();
    if root > 0 {
        parts.push(&path[..root]);
    }
    parts.extend(path[root..].split('/').filter(|part| !part.is_empty()));
    parts
}

pub(super) fn join<'a>(paths: impl IntoIterator<Item = &'a str>) -> String {
    let mut result = String::new();
    for path in paths {
        let root = root_len(path);
        if root > 0 {
            result.clear();
            result.push_str(&path[..root]);
        }
        for part in path[root..].split('/').filter(|part| !part.is_empty()) {
            if !result.is_empty() && !result.ends_with('/') {
                result.push('/');
            }
            result.push_str(part);
        }
    }
    result
}

fn file(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 0, "subcommand ?arg ...?")?;
    let operation = argv[1].as_str();
    if operation == "attributes" {
        return file_permissions::command(interp, id, argv);
    }
    if operation == "link" {
        return file_link::command(interp, id, argv);
    }
    if operation == "delete" {
        return file_delete::command(interp, id, argv);
    }
    if operation == "mkdir" {
        return filesystem_mutations::mkdir(interp, id, argv);
    }
    if matches!(operation, "mtime" | "atime") {
        return file_times::command(interp, id, argv);
    }
    if filesystem::is_metadata_query(operation) {
        return filesystem::metadata_query(interp, argv);
    }
    if filesystem::is_predicate(operation) {
        check_args(2, argv, 3, 3, "name")?;
        return Ok(filesystem::predicate(operation, argv[2].as_str()).into());
    }
    if operation == "join" {
        check_args(2, argv, 3, 0, "name ?name ...?")?;
        return Ok(join(argv[2..].iter().map(Value::as_str)).into());
    }
    if !matches!(
        operation,
        "dirname" | "tail" | "extension" | "rootname" | "split" | "pathtype" | "separator"
    ) {
        let name = format!("file {operation}");
        interp.context::<State>(id).unsupported.insert(name.clone());
        return Err(error(format!("unsupported Portfile command {name}")));
    }
    if operation == "separator" {
        check_args(2, argv, 2, 3, "?name?")?;
        return Ok("/".into());
    }
    check_args(2, argv, 3, 3, "name")?;
    let path = argv[2].as_str();
    match operation {
        "pathtype" => Ok(if path.starts_with('/') {
            "absolute"
        } else {
            "relative"
        }
        .into()),
        "extension" | "rootname" => {
            let extension = path
                .rfind('.')
                .filter(|&dot| !path[dot..].contains('/'))
                .unwrap_or(path.len());
            Ok(if operation == "extension" {
                &path[extension..]
            } else {
                &path[..extension]
            }
            .into())
        }
        "split" => Ok(Value::from(
            split(path).into_iter().map(Value::from).collect::<Vec<_>>(),
        )),
        "tail" => {
            let parts = split(path);
            Ok(if parts.len() == 1 && path.starts_with('/') {
                ""
            } else {
                parts.last().copied().unwrap_or("")
            }
            .into())
        }
        "dirname" => {
            let parts = split(path);
            Ok(if parts.len() > 1 {
                join(parts[..parts.len() - 1].iter().copied())
            } else if path.starts_with('/') {
                parts[0].to_owned()
            } else {
                ".".to_owned()
            }
            .into())
        }
        _ => unreachable!(),
    }
}
