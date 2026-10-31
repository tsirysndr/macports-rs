//! Native work-directory creation and migration of legacy MacPorts build trees.
use super::*;
use molt::{MoltResult, check_args};
use std::{
    ffi::CString,
    fs, io,
    os::unix::{ffi::OsStrExt, fs::symlink},
    path::{Path, PathBuf},
};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portutil::create_workpath", create, id);
    interp.add_context_command("chownAsRoot", chown_as_root, id);
    interp.add_context_command("elevateToRoot", elevate, id);
    interp.add_context_command("dropPrivileges", drop_privileges, id);
}
fn elevate(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "action")?;
    crate::privileges::elevate(argv[1].as_str()).map_err(|err| error(err.to_string()))?;
    Ok(Value::empty())
}
fn drop_privileges(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    let user = interp.with_global_scope(|interp| interp.scalar("macports::macportsuser"))?;
    if let Err(err) = crate::privileges::drop_to(user.as_str()) {
        eprintln!("Error: Failed to de-escalate privileges: {err}");
    }
    Ok(Value::empty())
}
fn execution(interp: &mut Interp, id: ContextID, name: &str) -> Result<(), Exception> {
    if interp.context::<State>(id).system_executor.is_none() {
        interp.context::<State>(id).unsupported.insert(name.into());
        return Err(error(format!("unsupported Portfile command {name}")));
    }
    Ok(())
}
fn call(interp: &mut Interp, words: &[&str]) -> MoltResult {
    interp.eval_value(&Value::from(
        words
            .iter()
            .map(|word| Value::from(*word))
            .collect::<Vec<_>>(),
    ))
}
fn create(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    execution(interp, id, "portutil::create_workpath")?;
    if crate::privileges::dropped() {
        crate::privileges::elevate("create_workpath").map_err(|err| error(err.to_string()))?;
    }
    interp.with_global_scope(|interp| {
        let work = interp.scalar("workpath")?.to_string();
        let build = interp.scalar("subbuildpath")?.to_string();
        let subport = interp.scalar("subport")?.to_string();
        let old_build = call(interp, &["portutil::get_oldbuildpath"])?;
        let old = paths::join([old_build.as_str(), &subport]);
        call(interp, &["file", "mkdir", old_build.as_str(), &build])?;
        let normalized = call(interp, &["file", "normalize", &format!("{old}/work")])?;
        if normalized.as_str() != work {
            if let Ok(kind) = call(interp, &["file", "type", &old]) {
                match kind.as_str() {
                    "directory" => {
                        call(interp, &["file", "delete", "-force", "--", &build])?;
                        call(interp, &["file", "rename", "--", &old, &build])?;
                    }
                    "link" if Path::new(&old).is_dir() => {
                        call(interp, &["file", "delete", "-force", "--", &build])?;
                        let source = call(interp, &["realpath", &old])?;
                        call(interp, &["file", "rename", "--", source.as_str(), &build])?;
                    }
                    _ => {
                        call(interp, &["file", "delete", "-force", "--", &old])?;
                    }
                }
                call(
                    interp,
                    &["file", "attributes", &build, "-permissions", "0755"],
                )?;
            }
            // ln -sf replaces the link itself and permits a relative target.
            match fs::remove_file(&old) {
                Ok(()) => (),
                Err(err) if err.kind() == io::ErrorKind::NotFound => (),
                Err(err) => {
                    return Err(filesystem::path_error(
                        &old,
                        "could not replace work link",
                        err,
                    ));
                }
            }
            symlink(&build, &old)
                .map_err(|err| filesystem::path_error(&old, "could not create work link", err))?;
            call(interp, &["chownAsRoot", &old])?;
        }
        call(
            interp,
            &[
                "file",
                "mkdir",
                &format!("{work}/.home"),
                &format!("{work}/.tmp"),
            ],
        )?;
        call(
            interp,
            &["file", "attributes", &work, "-permissions", "0755"],
        )?;
        call(interp, &["chownAsRoot", &build])
    })
}
fn chown_as_root(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "path")?;
    execution(interp, id, "chownAsRoot")?;
    // SAFETY: getuid has no memory preconditions and does not change identity.
    if unsafe { libc::getuid() } != 0 {
        return Ok(Value::empty());
    }
    interp.with_global_scope(|interp| {
        let user = interp.scalar("macports::macportsuser")?;
        let uid = if let Ok(value) = user.as_int() {
            value as u32
        } else {
            accounts::by_name(user.as_str(), false)
                .ok()
                .flatten()
                .map(|account| account.id)
                .ok_or_else(|| error(format!("unknown user {user}")))?
        };
        let path = PathBuf::from(argv[1].as_str());
        crate::privileges::as_root(user.as_str(), || {
            lchown(&path, uid)?;
            if path.is_dir() {
                traverse::visit_paths(vec![path], |path| lchown(path, uid))?;
            }
            Ok(Value::empty())
        })
    })
}
fn lchown(path: &Path, uid: u32) -> Result<(), Exception> {
    let native =
        CString::new(path.as_os_str().as_bytes()).map_err(|_| error("invalid ownership path"))?;
    // SAFETY: the path buffer is NUL terminated and live throughout the call;
    // the all-ones group sentinel preserves group ownership.
    if unsafe { libc::lchown(native.as_ptr(), uid, u32::MAX) } != 0 {
        return Err(filesystem::path_error(
            &path.to_string_lossy(),
            "could not set owner for file",
            io::Error::last_os_error(),
        ));
    }
    Ok(())
}
