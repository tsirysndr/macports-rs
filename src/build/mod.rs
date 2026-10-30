//! Source-phase subprocesses with immediate, labeled output. Command selection
//! and Portfile stage scheduling belong to the installer, not this runner.
mod log;
mod process;
use crate::progress::Progress;
use anyhow::Result;
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};
use tokio::io::AsyncWrite;

pub struct Command {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub directory: PathBuf,
    pub environment: BTreeMap<OsString, OsString>,
}
#[derive(Clone, Copy, Debug)]
pub enum Phase {
    Configure,
    Build,
    Test,
    Destroot,
}
impl Phase {
    pub fn label(self) -> &'static str {
        match self {
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
    let _paused = progress.pause_bars();
    let prefix = log::prefix(port, phase, progress.color_enabled());
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
    process::run(
        command,
        port,
        phase,
        &log::prefix(port, phase, false),
        output,
    )
    .await
}
