use super::{Command, Phase, ProcessResult, log::Log};
use anyhow::{Context, Result};
use std::process::Stdio;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};

/// A dedicated process group makes cancellation stop make and its compiler
/// children. Tokio's kill_on_drop also arranges reaping of the direct child.
struct ProcessGroup {
    pid: Option<u32>,
}
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if let Some(pid) = self.pid {
            // SAFETY: the PID came from our child created with process_group(0).
            // A negative PID signals that group, never our own process group.
            unsafe {
                libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
            }
        }
    }
}
pub(super) async fn run(
    command: &Command,
    port: &str,
    phase: Phase,
    prefix: &str,
    output: &mut (impl AsyncWrite + Unpin),
) -> Result<ProcessResult> {
    let mut builder = tokio::process::Command::new(&command.program);
    if command.clear_environment {
        builder.env_clear();
    }
    let mut child = builder
        .args(&command.args)
        .current_dir(&command.directory)
        .envs(&command.environment)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .with_context(|| {
            format!(
                "cannot start {} {port}: {}",
                phase.label(),
                command.program.to_string_lossy()
            )
        })?;
    let pid = child.id().expect("spawned child has a PID");
    let mut group = ProcessGroup { pid: Some(pid) };
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let result = tokio::try_join!(pump(stdout, stderr, output, prefix), child.wait());
    match result {
        Ok(((), status)) => {
            group.pid = None;
            Ok(ProcessResult { pid, status })
        }
        Err(error) => {
            drop(group);
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(error).with_context(|| format!("{} {port}: streaming build output", phase.label()))
        }
    }
}
async fn pump(
    mut stdout: impl AsyncRead + Unpin,
    mut stderr: impl AsyncRead + Unpin,
    output: &mut (impl AsyncWrite + Unpin),
    prefix: &str,
) -> std::io::Result<()> {
    let mut log = Log::new(output, prefix);
    let (mut out, mut err) = ([0u8; 8192], [0u8; 8192]);
    let (mut out_open, mut err_open) = (true, true);
    while out_open || err_open {
        tokio::select! {
            count = stdout.read(&mut out), if out_open => {
                let count = count?;
                if count == 0 { out_open = false; } else { log.write(false, &out[..count]).await?; }
            }
            count = stderr.read(&mut err), if err_open => {
                let count = count?;
                if count == 0 { err_open = false; } else { log.write(true, &err[..count]).await?; }
            }
        }
    }
    log.finish().await
}
