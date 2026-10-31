//! Compose native staging cleanup in PortSystem finish order.
use super::*;
use molt::MoltResult;
#[cfg(target_os = "macos")]
use std::fs;
use std::path::Path;

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portdestroot::destroot_finish", finish, id);
}
fn call(interp: &mut Interp, words: &[Value]) -> MoltResult {
    interp.eval_value(&Value::from(words.to_vec()))
}
fn finish(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        // Installed files must not belong to the unprivileged build account.
        if unsafe { libc::getuid() } == 0 {
            let user = interp.scalar("macports::macportsuser")?;
            let destroot = interp.scalar("destroot")?;
            let owner = interp.scalar("install.user")?;
            let group = interp.scalar("install.group")?;
            fix_ownership(
                Path::new(destroot.as_str()),
                user.as_str(),
                owner.as_str(),
                group.as_str(),
            )
            .map_err(|err| error(err.to_string()))?;
        }
        let oldmask = interp.scalar("portdestroot::oldmask")?;
        if interp.context::<State>(id).creation_mask.is_none() {
            return Err(error("creation mask is not initialized"));
        }
        let result = (|| -> Result<()> {
            let mut context = StageContext { interp, id };
            let destroot = context.scalar("destroot")?;
            let prefix = context.scalar("prefix")?;
            for name in ["share/info/dir", "lib/charset.alias"] {
                let path = format!("{destroot}{prefix}/{name}");
                if Path::new(&path).exists() {
                    call(
                        context.interp,
                        &["file".into(), "delete".into(), "--".into(), path.into()],
                    )
                    .map_err(failure)?;
                }
            }
            context.clean_destroot_libtool()?;
            context.prune_destroot_directories()?;
            context.compress_destroot_manpages()?;
            context.check_destroot_layout()?;
            if let Ok(work) = context.scalar("workpath") {
                // The upstream filesystem workaround is best effort; failure
                // must not turn an otherwise completed stage into a failure.
                let _ = sparse_workaround(Path::new(&destroot), Path::new(&work));
            }
            Ok(())
        })();
        // Restore even on failure; cleanup must not leak a staging mask into
        // later work. Keep the original failure if restoration also fails.
        let restored = call(interp, &["umask".into(), oldmask]);
        result.map_err(|err| error(err.to_string()))?;
        restored?;
        Ok(0.into())
    })
}

#[cfg(target_os = "macos")]
fn sparse_workaround(root: &Path, work: &Path) -> Result<()> {
    use std::{
        ffi::CString,
        os::unix::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawFd},
    };
    let mut temporary = None;
    traverse::visit_paths(vec![root.to_owned()], |path| {
        let metadata = fs::symlink_metadata(path).map_err(|err| error(err.to_string()))?;
        if !metadata.is_file() {
            return Ok(());
        }
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .map_err(|err| error(err.to_string()))?;
        // SAFETY: file owns the descriptor; lseek changes only its offset.
        let (hole, end) = unsafe {
            (
                libc::lseek(file.as_raw_fd(), 0, libc::SEEK_HOLE),
                libc::lseek(file.as_raw_fd(), 0, libc::SEEK_END),
            )
        };
        if hole < 0 || end < 0 || hole >= end {
            return Ok(());
        }
        if temporary.is_none() {
            temporary = Some(tempfile::tempdir_in(work).map_err(|err| error(err.to_string()))?);
        }
        let destination = temporary.as_ref().unwrap().path().join("sparse-workaround");
        let source =
            CString::new(path.as_os_str().as_bytes()).map_err(|err| error(err.to_string()))?;
        let target = CString::new(destination.as_os_str().as_bytes())
            .map_err(|err| error(err.to_string()))?;
        // SAFETY: both paths are live NUL-terminated strings, and the target
        // resides in our private temporary directory. No existing file is replaced.
        if unsafe { libc::clonefile(source.as_ptr(), target.as_ptr(), 0) } != 0 {
            return Err(error(std::io::Error::last_os_error().to_string()));
        }
        fs::remove_file(destination).map_err(|err| error(err.to_string()))?;
        Ok(())
    })
    .map_err(failure)?;
    Ok(())
}
#[cfg(not(target_os = "macos"))]
fn sparse_workaround(_: &Path, _: &Path) -> Result<()> {
    Ok(())
}

/// Give files owned by the build account to `install.user` and `install.group`,
/// keeping their permissions (upstream `destroot_finish`).
fn fix_ownership(destroot: &Path, user: &str, owner: &str, group: &str) -> Result<()> {
    use anyhow::Context;
    use std::os::unix::{ffi::OsStrExt, fs::MetadataExt, fs::PermissionsExt};
    let Some((build_uid, build_gid)) = crate::privileges::account(user) else {
        return Ok(());
    };
    let owner = crate::privileges::account(owner)
        .map(|(uid, _)| uid)
        .context("unknown install.user")?;
    let group = group
        .parse::<u32>()
        .ok()
        .or_else(|| {
            crate::accounts::by_name(group, true)
                .ok()
                .flatten()
                .map(|account| account.id)
        })
        .context("unknown install.group")?;
    traverse::visit_paths(vec![destroot.to_owned()], |path| {
        let metadata = std::fs::symlink_metadata(path).map_err(|err| error(err.to_string()))?;
        if metadata.uid() != build_uid && metadata.gid() != build_gid {
            return Ok(());
        }
        let uid = if metadata.uid() == build_uid {
            owner
        } else {
            u32::MAX
        };
        let gid = if metadata.gid() == build_gid {
            group
        } else {
            u32::MAX
        };
        let native = std::ffi::CString::new(path.as_os_str().as_bytes())
            .map_err(|err| error(err.to_string()))?;
        // SAFETY: the path is NUL terminated and live; -1 keeps an id unchanged.
        if unsafe { libc::lchown(native.as_ptr(), uid, gid) } != 0 {
            return Err(error(std::io::Error::last_os_error().to_string()));
        }
        // Changing owners may clear setuid bits; links carry no permissions.
        if !metadata.file_type().is_symlink() {
            std::fs::set_permissions(
                path,
                std::fs::Permissions::from_mode(metadata.permissions().mode()),
            )
            .map_err(|err| error(err.to_string()))?;
        }
        Ok(())
    })
    .map_err(failure)?;
    Ok(())
}
