//! Upstream's privilege lifecycle (portutil's `dropPrivileges`,
//! `elevateToRoot` and `chownAsRoot`): started as root, MacPorts runs build
//! work with the effective ids of `macportsuser` and returns to root to stage,
//! install and activate. The ids are process-wide, as in upstream.
use anyhow::{Result, bail};
use std::{ffi::CString, sync::OnceLock};

static STARTUP: OnceLock<(u32, u32)> = OnceLock::new();

/// Remember the effective ids that elevation restores. Call before any drop.
pub fn record_startup() -> (u32, u32) {
    // SAFETY: identity queries have no preconditions.
    *STARTUP.get_or_init(|| unsafe { (libc::geteuid(), libc::getegid()) })
}

/// Started as root, but currently running as the build account.
pub fn dropped() -> bool {
    // SAFETY: identity queries have no preconditions.
    unsafe { libc::getuid() == 0 && libc::geteuid() != 0 }
}

/// A user's uid and primary gid (upstream `name_to_uid`, `uname_to_gid`).
pub fn account(user: &str) -> Option<(u32, u32)> {
    if let Ok(uid) = user.parse::<u32>() {
        // SAFETY: getpwuid returns static storage, read before any other call.
        let record = unsafe { libc::getpwuid(uid) };
        return (!record.is_null()).then(|| unsafe { ((*record).pw_uid, (*record).pw_gid) });
    }
    let name = CString::new(user).ok()?;
    // SAFETY: the name is NUL terminated; the record is read immediately.
    let record = unsafe { libc::getpwnam(name.as_ptr()) };
    (!record.is_null()).then(|| unsafe { ((*record).pw_uid, (*record).pw_gid) })
}

/// Switch the effective ids to `user` when running as root.
pub fn drop_to(user: &str) -> Result<()> {
    record_startup();
    // SAFETY: identity queries have no preconditions.
    if unsafe { libc::geteuid() } != 0 {
        return Ok(());
    }
    let Some((uid, gid)) = account(user) else {
        bail!("Failed to de-escalate privileges: unknown user {user}");
    };
    if uid == 0 {
        return Ok(());
    }
    // SAFETY: plain credential changes; the group goes first while still root.
    if unsafe { libc::setegid(gid) != 0 || libc::seteuid(uid) != 0 } {
        bail!(
            "Failed to de-escalate privileges: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

/// Return to the startup ids for `action`; without root, fail as upstream.
pub fn elevate(action: &str) -> Result<()> {
    if dropped() {
        let (uid, gid) = record_startup();
        // SAFETY: the real uid is root, so the saved root id can be restored.
        if unsafe { libc::seteuid(uid) != 0 || libc::setegid(gid) != 0 } {
            bail!(
                "Failed to elevate privileges for {action}: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(())
    } else if unsafe { libc::getuid() } != 0 {
        bail!("MacPorts requires root privileges for this action")
    } else {
        Ok(())
    }
}

/// Run `body` with the startup ids, then return to `user` if privileges
/// were dropped before.
pub fn as_root<T>(user: &str, body: impl FnOnce() -> T) -> T {
    let was_dropped = dropped();
    if was_dropped {
        let _ = elevate("");
    }
    let result = body();
    if was_dropped {
        let _ = drop_to(user);
    }
    result
}
