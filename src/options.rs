//! Global options from the command line (upstream `ui_options` and
//! `global_options`), shared by every action of an invocation.
use std::sync::RwLock;

#[derive(Clone, Debug, Default)]
pub struct Global {
    /// `-d`: show debug messages (implies verbose).
    pub debug: bool,
    /// `-v`: show informational messages.
    pub verbose: bool,
    /// `-N` (and `-q`): never prompt.
    pub noninteractive: bool,
    /// `-p`: keep going after errors.
    pub process_all: bool,
    /// `-f`: force actions that would otherwise refuse.
    pub force: bool,
    /// `-o`: reuse build state even if the Portfile changed.
    pub ignore_different: bool,
    /// `-n`: do not act on dependencies.
    pub nodeps: bool,
    /// `-u`: uninstall replaced inactive images after upgrade.
    pub uninstall_old: bool,
    /// `-R`: also upgrade dependents.
    pub dependents: bool,
    /// `-c` / `-k`: clean work directories after install, or keep them.
    pub autoclean: Option<bool>,
    /// `-t`: trace mode.
    pub trace: bool,
    /// `-y`: dry run.
    pub dry_run: bool,
}

static GLOBAL: RwLock<Option<Global>> = RwLock::new(None);

pub fn set(options: Global) {
    *GLOBAL.write().expect("options lock") = Some(options);
}

pub fn get() -> Global {
    GLOBAL
        .read()
        .expect("options lock")
        .clone()
        .unwrap_or_default()
}
