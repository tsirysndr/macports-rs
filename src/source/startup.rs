//! Process configuration captured before the CLI starts concurrent work.
use std::sync::OnceLock;
static CREATION_MASK: OnceLock<u32> = OnceLock::new();

/// Capture the inherited mask once, restoring it immediately. Use the most
/// restrictive temporary mask so an unexpected signal cannot create a file
/// with more permissive access during capture.
///
/// # Safety
/// Call at process startup before any threads or signal handlers can create
/// files or change the process umask. Library embedders that cannot ensure this
/// must initialize individual Portfile sessions with an explicit mask instead.
pub unsafe fn capture_creation_mask_before_threads() -> u32 {
    *CREATION_MASK.get_or_init(|| {
        // SAFETY: caller guarantees exclusive access to process creation state;
        // both calls are adjacent and no allocation or fallible code intervenes.
        let previous = unsafe { libc::umask(0o777) };
        unsafe {
            libc::umask(previous);
        }
        u32::from(previous)
    })
}

/// The startup mask, or None when an embedding host has not captured it.
pub fn startup_creation_mask() -> Option<u32> {
    CREATION_MASK.get().copied()
}
