//! Explicitly enabled PortSystem shell execution; never enabled for metadata queries.
use super::*;
use crate::build::{Command, ProcessResult};
use anyhow::Context;
use molt::{MoltResult, check_args};
use std::{
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
};

pub(super) type Executor = Box<dyn FnMut(SystemRequest) -> Result<ProcessResult>>;

/// A single MacPorts `system` invocation, with its live interpreter environment.
#[derive(Debug)]
pub struct SystemRequest {
    pub script: String,
    pub directory: Option<PathBuf>,
    pub environment: BTreeMap<String, String>,
    /// Directory command_exec must create before launching its shell.
    pub prepare_directory: Option<PathBuf>,
    pub nice: Option<i32>,
    pub new_session: bool,
}
impl SystemRequest {
    /// MacPorts explicitly runs system scripts via /bin/sh -c. Resolve relative
    /// -W paths against the session's directory without changing the host cwd.
    /// For command_exec requests, prepare its directory relative to the same
    /// base before returning the executable command.
    pub fn into_command(self, directory: &Path) -> Result<Command> {
        if let Some(path) = &self.prepare_directory {
            let path = directory.join(path);
            if !path
                .try_exists()
                .with_context(|| format!("cannot inspect command directory {}", path.display()))?
            {
                std::fs::create_dir_all(&path).with_context(|| {
                    format!("cannot create command directory {}", path.display())
                })?;
            }
        }
        Ok(Command {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), self.script.into()],
            directory: self
                .directory
                .map(|path| directory.join(path))
                .unwrap_or_else(|| directory.into()),
            environment: self
                .environment
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
            clear_environment: true,
            nice: self.nice,
            new_session: self.new_session,
        })
    }
}

impl Session {
    /// Enable `system` with `-W`, `-notty`, and `-nice` after metadata
    /// evaluation. Supply the complete initial environment explicitly; changes
    /// to the Tcl env array are captured separately for every invocation.
    /// Installs lazy build-directory and compiler-environment defaults without
    /// replacing Portfile values. Host SDK/compiler discovery remains external.
    ///
    /// The executor is synchronous because Molt procedures are synchronous.
    /// An installer should own this session on a blocking worker and forward
    /// requests to its async build runner. Do not block a Tokio reactor thread.
    pub fn enable_system(
        &mut self,
        environment: &BTreeMap<String, String>,
        executor: impl FnMut(SystemRequest) -> Result<ProcessResult> + 'static,
    ) -> Result<()> {
        check_supported(&mut self.interp, self.id)?;
        // Use native array operations, not script interpolation. Preserve empty
        // env as an array so scripts may add/remove variables normally.
        self.interp.eval("unset -nocomplain env").map_err(failure)?;
        let values = environment
            .iter()
            .flat_map(|(k, v)| [Value::from(k.clone()), Value::from(v.clone())])
            .collect::<Vec<_>>();
        self.interp.array_set("env", &values).map_err(failure)?;
        build_defaults::initialize(&mut self.interp, self.id).map_err(failure)?;
        self.interp.context::<State>(self.id).system_executor = Some(Box::new(executor));
        self.interp.add_context_command("system", command, self.id);
        Ok(())
    }
}

fn command(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(
        1,
        argv,
        2,
        0,
        "?-notty? ?-nice value? ?-W directory? ?--? command",
    )?;
    let mut directory = None;
    let mut nice = None;
    let mut new_session = false;
    let mut i = 1;
    while i < argv.len() - 1 {
        match argv[i].as_str() {
            "-notty" => new_session = true,
            "-nice" => {
                i += 1;
                if i >= argv.len() - 1 {
                    return Err(error("system -nice requires a value and command"));
                }
                nice = parse_nice(&argv[i])?;
            }
            "-W" => {
                i += 1;
                if i >= argv.len() - 1 {
                    return Err(error("system -W requires a directory and command"));
                }
                if argv[i].as_str().is_empty() {
                    return Err(error("system -W directory must not be empty"));
                }
                directory = Some(PathBuf::from(argv[i].as_str()));
            }
            "--" if i == argv.len() - 2 => break,
            option => {
                // Output-routing and callback flags must not be silently
                // ignored, even when a Portfile catches the error.
                interp
                    .context::<State>(id)
                    .unsupported
                    .insert(format!("system {option}"));
                return Err(error(format!("unsupported system option {option}")));
            }
        }
        i += 1;
    }
    let environment = interp.with_global_scope(|interp| interp.array_get("env"));
    let request = SystemRequest {
        script: argv.last().unwrap().to_string(),
        directory,
        prepare_directory: None,
        nice,
        new_session,
        environment: environment
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| (pair[0].to_string(), pair[1].to_string()))
            .collect(),
    };
    execute_request(interp, id, request)
}

pub(super) fn execute_request(
    interp: &mut Interp,
    id: ContextID,
    request: SystemRequest,
) -> MoltResult {
    let result = interp
        .context::<State>(id)
        .system_executor
        .as_mut()
        .ok_or_else(|| error("system executor is not enabled"))?(request)
    .map_err(|err| error(format!("system execution: {err:#}")))?;
    if result.status.success() {
        return Ok(Value::empty());
    }
    let code = if let Some(code) = result.status.code() {
        list([
            "CHILDSTATUS".into(),
            result.pid.to_string(),
            code.to_string(),
        ])
    } else {
        let signal = result.status.signal().unwrap_or(0);
        let (name, description) = signal_description(signal);
        list([
            "CHILDKILLED".into(),
            result.pid.to_string(),
            name,
            description,
        ])
    };
    Err(Exception::molt_err2(
        code,
        "command execution failed".into(),
    ))
}

fn signal_description(signal: i32) -> (String, String) {
    let (name, description) = match signal {
        libc::SIGHUP => ("SIGHUP", "hangup"),
        libc::SIGINT => ("SIGINT", "interrupt"),
        libc::SIGQUIT => ("SIGQUIT", "quit signal"),
        libc::SIGILL => ("SIGILL", "illegal instruction"),
        libc::SIGTRAP => ("SIGTRAP", "trace trap"),
        libc::SIGABRT => ("SIGABRT", "SIGABRT"),
        libc::SIGFPE => ("SIGFPE", "floating-point exception"),
        libc::SIGKILL => ("SIGKILL", "kill signal"),
        libc::SIGBUS => ("SIGBUS", "bus error"),
        libc::SIGSEGV => ("SIGSEGV", "segmentation violation"),
        libc::SIGSYS => ("SIGSYS", "bad argument to system call"),
        libc::SIGPIPE => ("SIGPIPE", "write on pipe with no readers"),
        libc::SIGALRM => ("SIGALRM", "alarm clock"),
        libc::SIGTERM => ("SIGTERM", "software termination signal"),
        libc::SIGURG => ("SIGURG", "urgent I/O condition"),
        libc::SIGSTOP => ("SIGSTOP", "stop"),
        libc::SIGTSTP => ("SIGTSTP", "stop signal from tty"),
        libc::SIGCONT => ("SIGCONT", "continue after stop"),
        libc::SIGCHLD => ("SIGCHLD", "child status changed"),
        libc::SIGTTIN => ("SIGTTIN", "background tty read"),
        libc::SIGTTOU => ("SIGTTOU", "background tty write"),
        libc::SIGIO => ("SIGIO", "input/output possible on file"),
        libc::SIGXCPU => ("SIGXCPU", "exceeded CPU time limit"),
        libc::SIGXFSZ => ("SIGXFSZ", "exceeded file size limit"),
        libc::SIGVTALRM => ("SIGVTALRM", "virtual time alarm"),
        libc::SIGPROF => ("SIGPROF", "profiling alarm"),
        libc::SIGWINCH => ("SIGWINCH", "window changed"),
        libc::SIGUSR1 => ("SIGUSR1", "user-defined signal 1"),
        libc::SIGUSR2 => ("SIGUSR2", "user-defined signal 2"),
        #[cfg(target_os = "macos")]
        libc::SIGEMT => ("SIGEMT", "EMT instruction"),
        #[cfg(target_os = "macos")]
        libc::SIGINFO => ("SIGINFO", "information request"),
        _ => ("unknown signal", "unknown signal"),
    };
    (name.into(), description.into())
}

/// INT_MAX is the native system command's sentinel for no priority adjustment.
pub(super) fn parse_nice(value: &Value) -> Result<Option<i32>, Exception> {
    // Tcl_GetIntFromObj on macOS accepts INT_MIN..UINT_MAX, then casts to int.
    let number = value
        .as_int()
        .ok()
        .filter(|value| (i32::MIN as i64..=u32::MAX as i64).contains(value))
        .ok_or_else(|| error("invalid value for -nice"))? as i32;
    Ok((number != i32::MAX).then_some(number))
}
