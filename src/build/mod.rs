//! Source-phase subprocesses with immediate, labeled output. Command selection
//! and Portfile stage scheduling belong to the installer, not this runner.
pub(crate) mod directories;
mod log;
pub mod mtree;
mod process;
pub mod state;
use crate::progress::Progress;
use anyhow::{Result, ensure};
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf, process::ExitStatus};
use tokio::io::AsyncWrite;

pub struct Command {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub directory: PathBuf,
    pub environment: BTreeMap<OsString, OsString>,
    /// Clear inherited variables before applying `environment`.
    pub clear_environment: bool,
    /// Requested absolute niceness; permission failures follow MacPorts and are ignored.
    pub nice: Option<i32>,
    /// Start a session without a controlling terminal (`system -notty`).
    pub new_session: bool,
    /// Child-only file creation mask; None inherits the host mask.
    pub creation_mask: Option<u32>,
}
/// Completed process identity and status, retained for Tcl CHILDSTATUS errors.
#[derive(Debug)]
pub struct ProcessResult {
    pub pid: u32,
    pub status: ExitStatus,
}
impl ProcessResult {
    fn success(self, port: &str, phase: Phase) -> Result<()> {
        ensure!(
            self.status.success(),
            "{} {port} failed: {}",
            phase.label(),
            self.status
        );
        Ok(())
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Phase {
    Fetch,
    Checksum,
    Extract,
    Patch,
    Configure,
    Build,
    Test,
    Destroot,
}
impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Fetch => "fetching",
            Self::Checksum => "verifying",
            Self::Extract => "extracting",
            Self::Patch => "patching",
            Self::Configure => "configuring",
            Self::Build => "building",
            Self::Test => "testing",
            Self::Destroot => "staging",
        }
    }
}
/// Default source-build presentation: live logs on stderr, regardless of verbose
/// mode. Progress redraws are paused until the phase ends or is cancelled.
pub async fn run(command: &Command, port: &str, phase: Phase, progress: &Progress) -> Result<()> {
    run_status(command, port, phase, progress)
        .await?
        .success(port, phase)
}
/// Colored live output with raw process status for Portfile error handling.
pub async fn run_status(
    command: &Command,
    port: &str,
    phase: Phase,
    progress: &Progress,
) -> Result<ProcessResult> {
    let _paused = progress.pause_bars();
    let prefix = log::prefix(port, phase, progress.color_enabled());
    if progress.logging() {
        let mut stderr = tokio::io::stderr();
        let mut tee =
            crate::progress::LogTee::new(&mut stderr, progress, log::prefix(port, phase, false));
        return process::run(command, port, phase, &prefix, &mut tee).await;
    }
    process::run(command, port, phase, &prefix, &mut tokio::io::stderr()).await
}
/// Stream plain-text logs to a caller-owned sink. Each read is flushed, without
/// waiting for a newline or process exit. Dropping this future kills the build's
/// process group; a broken sink also stops the build instead of deadlocking it.
pub async fn run_to(
    command: &Command,
    port: &str,
    phase: Phase,
    output: &mut (impl AsyncWrite + Unpin),
) -> Result<()> {
    run_status_to(command, port, phase, output)
        .await?
        .success(port, phase)
}

/// Like `run_to`, but returns nonzero exit statuses for interpreter-level error
/// handling. Transport/spawn errors still fail, and all output is drained first.
pub async fn run_status_to(
    command: &Command,
    port: &str,
    phase: Phase,
    output: &mut (impl AsyncWrite + Unpin),
) -> Result<ProcessResult> {
    process::run(
        command,
        port,
        phase,
        &log::prefix(port, phase, false),
        output,
    )
    .await
}

pub mod work_state;
