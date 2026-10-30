use port_rs::build::{self, Command, Phase};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    io,
    path::Path,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWrite};
fn command(directory: &Path, script: &str) -> Command {
    Command {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        directory: directory.into(),
        environment: BTreeMap::new(),
        clear_environment: false,
        nice: None,
        new_session: false,
        creation_mask: None,
    }
}
#[tokio::test]
async fn partial_output_arrives_before_process_exit_on_both_streams() {
    for redirect in ["", " >&2"] {
        let directory = tempfile::tempdir().unwrap();
        let gate = directory.path().join("continue");
        let script = format!(
            "printf first{redirect}; while [ ! -e \"$BUILD_GATE\" ]; do sleep 0.01; done; printf '\\nlast\\n'{redirect}"
        );
        let mut command = command(directory.path(), &script);
        command
            .environment
            .insert("BUILD_GATE".into(), gate.as_os_str().into());
        let (writer, mut reader) = tokio::io::duplex(1024);
        let task = tokio::spawn(async move {
            let mut writer = tokio::io::BufWriter::with_capacity(8192, writer);
            build::run_to(&command, "git", Phase::Build, &mut writer).await
        });
        let expected = b"-> building git: first";
        let mut first = vec![0; expected.len()];
        tokio::time::timeout(Duration::from_secs(3), reader.read_exact(&mut first))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first, expected);
        assert!(
            !task.is_finished(),
            "output must arrive while the build is still running"
        );
        std::fs::write(gate, b"go").unwrap();
        let mut rest = Vec::new();
        reader.read_to_end(&mut rest).await.unwrap();
        task.await.unwrap().unwrap();
        assert_eq!(rest, b"\n-> building git: last\n");
    }
}
#[tokio::test]
async fn streams_stdout_and_stderr_and_preserves_failure_status() {
    let directory = tempfile::tempdir().unwrap();
    let command = command(
        directory.path(),
        "printf 'compiling\\n'; printf 'fatal: compile failed\\n' >&2; exit 7",
    );
    let mut output = Vec::new();
    let error = build::run_to(&command, "git", Phase::Build, &mut output)
        .await
        .unwrap_err();
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("-> building git: compiling\n"));
    assert!(output.contains("-> building git: fatal: compile failed\n"));
    assert!(error.to_string().contains("building git failed"));
    assert!(error.to_string().contains('7'));
}
#[tokio::test]
async fn large_simultaneous_streams_are_drained_without_deadlock() {
    let directory = tempfile::tempdir().unwrap();
    let command = command(
        directory.path(),
        "i=0; while [ $i -lt 20000 ]; do printf 'O\\n'; printf 'E\\n' >&2; i=$((i+1)); done",
    );
    let mut output = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(10),
        build::run_to(&command, "git", Phase::Build, &mut output),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(output.iter().filter(|&&b| b == b'O').count(), 20000);
    assert_eq!(output.iter().filter(|&&b| b == b'E').count(), 20000);
    for line in output
        .split(|&b| b == b'\n')
        .filter(|line| !line.is_empty())
    {
        assert!(line.starts_with(b"-> building git: "));
    }
}
#[tokio::test]
async fn uses_requested_directory_environment_and_literal_argv() {
    let directory = tempfile::tempdir().unwrap();
    let mut command = command(
        directory.path(),
        "printf '%s\\n' \"$PWD\" \"$BUILD_TEST_VALUE\"",
    );
    command
        .environment
        .insert("BUILD_TEST_VALUE".into(), "value with spaces".into());
    let mut output = Vec::new();
    build::run_to(&command, "git", Phase::Configure, &mut output)
        .await
        .unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(
        output.contains(
            &directory
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .to_string()
        )
    );
    assert!(output.contains("-> configuring git: value with spaces\n"));
    command.program = "/usr/bin/printf".into();
    command.args = vec![
        OsString::from("%s\n"),
        OsString::from("literal;$(echo must-not-expand)"),
    ];
    let mut output = Vec::new();
    build::run_to(&command, "git", Phase::Build, &mut output)
        .await
        .unwrap();
    assert_eq!(
        output,
        b"-> building git: literal;$(echo must-not-expand)\n"
    );
}
#[tokio::test]
async fn aborting_build_stops_the_child_process() {
    let directory = tempfile::tempdir().unwrap();
    let command = command(directory.path(), "printf '%s\\n' $$; exec sleep 30");
    let (mut writer, reader) = tokio::io::duplex(1024);
    let task =
        tokio::spawn(
            async move { build::run_to(&command, "git", Phase::Build, &mut writer).await },
        );
    let mut reader = tokio::io::BufReader::new(reader);
    let mut line = String::new();
    use tokio::io::AsyncBufReadExt;
    tokio::time::timeout(Duration::from_secs(3), reader.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let pid: i32 = line
        .trim()
        .strip_prefix("-> building git: ")
        .unwrap()
        .parse()
        .unwrap();
    task.abort();
    let _ = task.await;
    for _ in 0..100 {
        // SAFETY: signal 0 probes existence without changing the child process.
        if unsafe { libc::kill(pid, 0) } == -1
            && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("cancelled child {pid} was not reaped");
}
struct Broken;
impl AsyncWrite for Broken {
    fn poll_write(self: Pin<&mut Self>, _: &mut Context<'_>, _: &[u8]) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
#[tokio::test]
async fn broken_output_stops_build_and_missing_executable_is_reported() {
    let directory = tempfile::tempdir().unwrap();
    let mut command = command(directory.path(), "printf started; exec sleep 30");
    let error = tokio::time::timeout(
        Duration::from_secs(3),
        build::run_to(&command, "git", Phase::Build, &mut Broken),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(error.to_string().contains("streaming build output"));
    command.program = directory.path().join("missing-executable").into_os_string();
    let error = build::run_to(&command, "git", Phase::Build, &mut Vec::new())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("cannot start building git"));
}

#[tokio::test]
async fn cancellation_also_stops_compiler_children_in_the_build_group() {
    for new_session in [false, true] {
        assert_compiler_child_cancelled(new_session).await;
    }
}
async fn assert_compiler_child_cancelled(new_session: bool) {
    let directory = tempfile::tempdir().unwrap();
    let mut command = command(
        directory.path(),
        "sleep 30 & compiler=$!; printf '%s\\n' $compiler; wait",
    );
    command.new_session = new_session;
    let (mut writer, reader) = tokio::io::duplex(1024);
    let task =
        tokio::spawn(
            async move { build::run_to(&command, "git", Phase::Build, &mut writer).await },
        );
    let mut reader = tokio::io::BufReader::new(reader);
    let mut line = String::new();
    use tokio::io::AsyncBufReadExt;
    tokio::time::timeout(Duration::from_secs(3), reader.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let pid: i32 = line
        .trim()
        .strip_prefix("-> building git: ")
        .unwrap()
        .parse()
        .unwrap();
    task.abort();
    let _ = task.await;
    for _ in 0..300 {
        // SAFETY: signal 0 only probes the known compiler-child PID.
        if unsafe { libc::kill(pid, 0) } == -1
            && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("cancelled compiler child {pid} is still present");
}

#[tokio::test]
async fn requested_session_and_priority_apply_only_to_the_child() {
    use tokio::io::AsyncBufReadExt;
    // SAFETY: querying this process's identifiers and priority changes no state.
    let (parent_session, parent_priority) =
        unsafe { (libc::getsid(0), libc::getpriority(libc::PRIO_PROCESS, 0)) };
    for new_session in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut command = command(directory.path(), "printf '%s\\n' $$; exec /bin/sleep 30");
        command.nice = Some(19);
        command.new_session = new_session;
        let (mut writer, reader) = tokio::io::duplex(1024);
        let task =
            tokio::spawn(
                async move { build::run_to(&command, "git", Phase::Build, &mut writer).await },
            );
        let mut reader = tokio::io::BufReader::new(reader);
        let mut line = String::new();
        let read = tokio::time::timeout(Duration::from_secs(3), reader.read_line(&mut line)).await;
        let pid = line
            .trim()
            .strip_prefix("-> building git: ")
            .and_then(|pid| pid.parse::<i32>().ok());
        let metadata = pid.map(|pid| {
            // SAFETY: read-only queries of the child PID emitted by our fixture.
            unsafe {
                (
                    libc::getsid(pid),
                    libc::getpgid(pid),
                    libc::getpriority(libc::PRIO_PROCESS, pid as _),
                )
            }
        });
        task.abort();
        let _ = task.await;
        read.unwrap().unwrap();
        let pid = pid.unwrap();
        let (session, group, priority) = metadata.unwrap();
        assert_eq!(session, if new_session { pid } else { parent_session });
        assert_eq!(group, pid);
        assert!(priority >= 19, "child priority: {priority}");
    }
    // SAFETY: read-only queries verify no parent priority/session mutation.
    unsafe {
        assert_eq!(libc::getsid(0), parent_session);
        assert_eq!(libc::getpriority(libc::PRIO_PROCESS, 0), parent_priority);
    }
}

#[tokio::test]
async fn concurrent_children_use_independent_creation_masks() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let mut private = command(root.path(), ": > private");
    private.creation_mask = Some(0o077);
    let mut public = command(root.path(), ": > public");
    public.creation_mask = Some(0o022);
    let (mut private_log, mut public_log) = (Vec::new(), Vec::new());
    let (a, b) = tokio::join!(
        build::run_to(&private, "one", Phase::Build, &mut private_log),
        build::run_to(&public, "two", Phase::Build, &mut public_log)
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(
        std::fs::metadata(root.path().join("private"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(root.path().join("public"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
}
