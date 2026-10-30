//! Native Tcl link queries and execution-only link creation.
use super::*;
use molt::{MoltResult, check_args};
use std::{fs, io, os::unix::fs::symlink, path::Path};

pub(super) fn command(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(2, argv, 3, 5, "?-linktype? linkname ?target?")?;
    if argv.len() == 3 {
        return filesystem::metadata_query(
            interp,
            &[argv[0].clone(), "readlink".into(), argv[2].clone()],
        );
    }
    let (symbolic, index) = if argv.len() == 5 {
        let option = argv[2].as_str();
        let matches: Vec<_> = ["-symbolic", "-hard"]
            .into_iter()
            .filter(|name| name.starts_with(option))
            .collect();
        if matches.len() != 1 {
            return Err(Exception::molt_err2(
                Value::from(vec![
                    "TCL".into(),
                    "LOOKUP".into(),
                    "INDEX".into(),
                    "option".into(),
                    option.into(),
                ]),
                format!(
                    "{} option \"{option}\": must be -symbolic or -hard",
                    if matches.is_empty() {
                        "bad"
                    } else {
                        "ambiguous"
                    }
                )
                .into(),
            ));
        }
        (matches[0] == "-symbolic", 3)
    } else {
        (true, 2)
    };
    if interp.context::<State>(id).system_executor.is_none() {
        interp
            .context::<State>(id)
            .unsupported
            .insert("file link creation".into());
        return Err(error("unsupported Portfile command file link creation"));
    }
    let name = argv[index].as_str();
    let target = argv[index + 1].as_str();
    let native_name = paths::join([name]);
    let parent = Path::new(&native_name)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let lookup = if symbolic && !Path::new(target).is_absolute() {
        parent.join(target)
    } else {
        Path::new(target).to_owned()
    };
    // Tcl requires an existing target, including for symbolic links, and tests
    // relative symbolic targets from the link's parent rather than the cwd.
    let result = if !filesystem::predicate("exists", lookup.to_str().unwrap_or("")) {
        Err(io::Error::from_raw_os_error(libc::ENOENT))
    } else if symbolic {
        symlink(target, &native_name)
    } else {
        fs::hard_link(target, &native_name)
    };
    result.map_err(|err| link_error(name, target, parent, err))?;
    Ok(argv[index + 1].clone())
}

fn link_error(name: &str, target: &str, parent: &Path, err: io::Error) -> Exception {
    let code = err.raw_os_error();
    let detail = match code {
        Some(libc::EEXIST) => Some("that path already exists".to_owned()),
        Some(libc::ENOENT) if filesystem::predicate("exists", parent.to_str().unwrap_or("")) => {
            Some(format!("target \"{target}\" doesn't exist"))
        }
        Some(libc::ENOENT) => Some("no such file or directory".to_owned()),
        _ => None,
    };
    let exception = filesystem::path_error(
        &format!("{name}\" pointing to \"{target}"),
        "could not create new link",
        err,
    );
    if let Some(detail) = detail {
        Exception::molt_err2(
            exception.error_code(),
            format!("could not create new link \"{name}\": {detail}").into(),
        )
    } else {
        exception
    }
}
