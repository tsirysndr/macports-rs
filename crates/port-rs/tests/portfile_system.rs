use port_rs::{
    build::{self, Phase},
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::AsyncReadExt;

fn open(source: &str) -> Session {
    Session::open(
        source,
        &BTreeMap::new(),
        &PortSpec::named("git"),
        PortGroups::default(),
    )
    .unwrap()
}

#[test]
fn system_is_disabled_during_metadata_evaluation_even_if_caught() {
    assert!(
        Session::open(
            "name git; version 1; catch {system {exit 0}}",
            &BTreeMap::new(),
            &PortSpec::named("git"),
            PortGroups::default()
        )
        .is_err()
    );
}

#[test]
fn executes_shell_in_requested_directory_with_live_isolated_environment() {
    let tree = tempfile::tempdir().unwrap();
    let child = tree.path().join("directory with spaces");
    std::fs::create_dir(&child).unwrap();
    let mut session = open(
        r#"
        name git; version 1
        proc commands {} {
            global env
            set env(MESSAGE) {hello world}
            unset env(REMOVED)
            system -W {directory with spaces} {printf '%s\n' "$PWD" "$MESSAGE" "${REMOVED-unset}" "${HOME-unset}"; printf 'stderr\n' >&2}
        }
    "#,
    );
    let output = Arc::new(Mutex::new(Vec::new()));
    let sink = output.clone();
    let directory = tree.path().to_owned();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    session
        .enable_system(
            &BTreeMap::from([("REMOVED".into(), "initial".into())]),
            move |request| {
                let mut output = sink.lock().unwrap();
                runtime.block_on(build::run_status_to(
                    &request.into_command(&directory)?,
                    "git",
                    Phase::Build,
                    &mut *output,
                ))
            },
        )
        .unwrap();
    assert_eq!(session.invoke("commands", &[]).unwrap(), "");
    let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    assert!(output.contains(&format!(
        "-> building git: {}\n",
        child.canonicalize().unwrap().display()
    )));
    assert!(output.contains("-> building git: hello world\n"));
    assert_eq!(output.matches("-> building git: unset\n").count(), 2);
    assert!(output.contains("-> building git: stderr\n"));
}

#[test]
fn catch_receives_childstatus_and_signaled_errors_and_can_recover() {
    let tree = tempfile::tempdir().unwrap();
    let directory = tree.path().to_owned();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut session = open(
        r#"
        name git; version 1
        proc failed {script} {
            global errorCode
            set code [catch {system $script} message]
            return [list $code $message $errorCode]
        }
    "#,
    );
    session
        .enable_system(&BTreeMap::new(), move |request| {
            runtime.block_on(build::run_status_to(
                &request.into_command(&directory)?,
                "git",
                Phase::Build,
                &mut Vec::new(),
            ))
        })
        .unwrap();
    let result = port_rs::tcl_list::parse(&session.invoke("failed", &["exit 7"]).unwrap()).unwrap();
    assert_eq!(&result[..2], &["1", "command execution failed"]);
    let code = port_rs::tcl_list::parse(&result[2]).unwrap();
    assert_eq!(code[0], "CHILDSTATUS");
    assert!(code[1].parse::<u32>().unwrap() > 0);
    assert_eq!(code[2], "7");
    let result =
        port_rs::tcl_list::parse(&session.invoke("failed", &["kill -TERM $$"]).unwrap()).unwrap();
    let code = port_rs::tcl_list::parse(&result[2]).unwrap();
    assert_eq!(code[0], "CHILDKILLED");
    assert_eq!(code[2], "SIGTERM");
    assert_eq!(code[3], "software termination signal");
    assert_eq!(session.invoke("system", &["--", "exit 0"]).unwrap(), "");
}

#[test]
fn rejects_unimplemented_flags_before_dispatch_even_if_script_catches_them() {
    for flag in ["-callback", "-nodup", "-unknown"] {
        let mut session =
            open("name git; version 1; proc call {flag} {catch {system $flag {exit 0}}}");
        session
            .enable_system(&BTreeMap::new(), |_| panic!("must not execute"))
            .unwrap();
        assert!(session.invoke("call", &[flag]).is_err());
    }
    let mut session = open("name git; version 1");
    session
        .enable_system(&BTreeMap::new(), |_| panic!("must not execute"))
        .unwrap();
    assert!(
        session
            .invoke("system", &["-W", "missing-command"])
            .is_err()
    );
}

#[tokio::test]
async fn portfile_system_streams_partial_logs_before_shell_exits() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_owned();
    let gate = path.join("continue");
    let (mut writer, mut reader) = tokio::io::duplex(1024);
    let worker = tokio::task::spawn_blocking(move || {
        // Construct Molt on its owning worker: it is intentionally not Send.
        let mut session = open(
            r#"
            name git; version 1
            proc compile {} {
                system {printf compiling; while [ ! -e continue ]; do /bin/sleep 0.01; done; printf '\nfinished\n'}
            }
        "#,
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        session
            .enable_system(&BTreeMap::new(), move |request| {
                runtime.block_on(build::run_status_to(
                    &request.into_command(&path)?,
                    "git",
                    Phase::Build,
                    &mut writer,
                ))
            })
            .unwrap();
        session.invoke("compile", &[])
    });
    let expected = b"-> building git: compiling";
    let mut first = vec![0; expected.len()];
    let observed =
        tokio::time::timeout(Duration::from_secs(5), reader.read_exact(&mut first)).await;
    let running = !worker.is_finished();
    // Release the fixture even on an assertion failure, so the worker cannot hang.
    std::fs::write(gate, b"go").unwrap();
    observed.unwrap().unwrap();
    assert_eq!(first, expected);
    assert!(running);
    let mut rest = Vec::new();
    reader.read_to_end(&mut rest).await.unwrap();
    assert_eq!(rest, b"\n-> building git: finished\n");
    worker.await.unwrap().unwrap();
}

#[test]
fn native_scheduler_stops_dependent_shell_commands_after_build_failure() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_owned();
    let output = Arc::new(Mutex::new(Vec::new()));
    let sink = output.clone();
    let mut session = open(
        r#"
        name git; version 1
        set compile [target_new compile compile]
        ditem_key $compile provides compiled
        set install [target_new install install_files]
        target_requires $install compiled
        proc compile {name} {system {printf 'compiler failed\n'; exit 9}}
        proc install_files {name} {system {printf wrong > must-not-exist}}
        proc handler {item} {
            set procedure [ditem_key $item procedure]
            $procedure [ditem_key $item name]
        }
        proc schedule {} {
            global compile install
            set remaining [dlist_eval [list $install $compile] {} handler]
            return [list [llength $remaining] $dlist_eval_reason]
        }
    "#,
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    session
        .enable_system(&BTreeMap::new(), move |request| {
            let mut output = sink.lock().unwrap();
            runtime.block_on(build::run_status_to(
                &request.into_command(&path)?,
                "git",
                Phase::Build,
                &mut *output,
            ))
        })
        .unwrap();
    assert_eq!(session.invoke("schedule", &[]).unwrap(), "2 handler");
    assert!(!directory.path().join("must-not-exist").exists());
    assert_eq!(
        output.lock().unwrap().as_slice(),
        b"-> building git: compiler failed\n"
    );
}

#[test]
fn system_parses_session_and_priority_options_with_native_integer_bounds() {
    use std::os::unix::process::ExitStatusExt;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let mut session = open("name git; version 1");
    session
        .enable_system(&BTreeMap::new(), move |request| {
            captured
                .lock()
                .unwrap()
                .push((request.new_session, request.nice));
            Ok(build::ProcessResult {
                pid: 1,
                status: std::process::ExitStatus::from_raw(0),
            })
        })
        .unwrap();
    for (value, expected) in [
        ("19", Some(19)),
        ("-20", Some(-20)),
        ("0xffffffff", Some(-1)),
        ("2147483647", None),
        ("0x80000000", Some(i32::MIN)),
    ] {
        session
            .invoke("system", &["-nice", value, "-notty", "exit 0"])
            .unwrap();
        assert_eq!(
            requests.lock().unwrap().last().copied(),
            Some((true, expected))
        );
    }
    for value in ["invalid", "1.5", "4294967296", "-2147483649"] {
        assert!(
            session
                .invoke("system", &["-nice", value, "exit 0"])
                .is_err()
        );
    }
    assert!(session.invoke("system", &["-nice", "exit 0"]).is_err());
    assert_eq!(requests.lock().unwrap().len(), 5);
    session
        .invoke("system", &["-nice", "5", "-nice", "8", "exit 0"])
        .unwrap();
    assert_eq!(
        requests.lock().unwrap().last().copied(),
        Some((false, Some(8)))
    );
}

#[test]
fn enabling_execution_can_preserve_environment_changes_from_evaluation() {
    use std::os::unix::process::ExitStatusExt;
    let mut session = Session::open_with_environment(
        "name git; version 1; unset env(REMOVE); set env(NEW) changed",
        &BTreeMap::new(),
        &PortSpec::named("git"),
        PortGroups::default(),
        &BTreeMap::from([
            ("REMOVE".into(), "initial".into()),
            ("PATH".into(), "/explicit/bin".into()),
        ]),
    )
    .unwrap();
    session
        .enable_system_with_current_environment(|request| {
            assert_eq!(request.environment["PATH"], "/explicit/bin");
            assert_eq!(request.environment["NEW"], "changed");
            assert!(!request.environment.contains_key("REMOVE"));
            assert!(!request.environment.contains_key("HOME"));
            Ok(build::ProcessResult {
                pid: 1,
                status: std::process::ExitStatus::from_raw(0),
            })
        })
        .unwrap();
    session
        .invoke("system", &["not actually executed"])
        .unwrap();
}
