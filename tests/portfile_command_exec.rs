use port_rs::{
    build::{self, Phase},
    portfile::{PortGroups, Session},
    spec::PortSpec,
    tcl_list,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
fn open(body: &str) -> Session {
    Session::open(
        &format!(
            r#"
        name app; version 1
        set macosx_deployment_target 15.0
        set configure.sdkroot {{/SDK path}}
        set configure.developer_dir {{/Developer path}}
        set source_date_epoch 12345
        set compiler.log_verbose_output no
        set compiler.cpath [list /include/one {{/include/two words}}]
        set compiler.library_path [list /lib/one /lib/two]
        {body}
    "#
        ),
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap()
}
#[test]
fn command_strings_join_shell_fragments_without_reinterpreting_tcl() {
    let mut session = open(
        r#"
        commands custom
        option custom.dir {/some path}
        custom.cmd tool {--flag="two words"}
        custom.pre_args before
        custom.args {"$SHELL_VAR"} {[not-a-tcl-command]}
        custom.post_args {> output}
        proc get_command {} {set custom.args local; return [command_string custom]}
    "#,
    );
    assert_eq!(
        session.invoke("get_command", &[]).unwrap(),
        "cd \"/some path\" && tool --flag=\"two words\" before \"$SHELL_VAR\" [not-a-tcl-command] > output"
    );
    assert_eq!(
        session.invoke("command_string", &["absent"]).unwrap(),
        " absent"
    );
}
#[test]
fn environment_helpers_parse_first_equals_and_append_nonempty_values() {
    let mut session = open(
        r#"
        commands custom
        custom.env A=one BROKEN {B=two=three} A=last EMPTY=
        parse_environment custom
        set result [append_to_environment_value custom A {} {next value}]
        set missing [append_to_environment_value custom MISSING {}]
        proc render {} {array set local {Z last A first}; return [environment_array_to_string local]}
    "#,
    );
    assert_eq!(session.scalar("result").unwrap(), "0");
    assert_eq!(session.scalar("missing").unwrap(), "1");
    assert_eq!(
        session
            .invoke("environment_array_to_string", &["custom.env_array"])
            .unwrap(),
        "\nA='last next value'\nB='two=three'\nEMPTY=''"
    );
    assert_eq!(
        session.invoke("render", &[]).unwrap(),
        "\nA='first'\nZ='last'"
    );
}
#[test]
fn metadata_queries_cannot_execute_commands_even_when_errors_are_caught() {
    assert!(
        Session::open(
            "name app; version 1; catch {command_exec build}",
            &BTreeMap::new(),
            &PortSpec::named("app"),
            PortGroups::default()
        )
        .is_err()
    );
}

fn executor(session: &mut Session, directory: std::path::PathBuf) -> Arc<Mutex<Vec<u8>>> {
    let output = Arc::new(Mutex::new(Vec::new()));
    let sink = output.clone();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    session
        .enable_system(
            &BTreeMap::from([("KEEP".into(), "original".into())]),
            move |request| {
                let mut output = sink.lock().unwrap();
                runtime.block_on(build::run_status_to(
                    &request.into_command(&directory)?,
                    "app",
                    Phase::Build,
                    &mut *output,
                ))
            },
        )
        .unwrap();
    output
}

#[test]
fn command_execution_creates_directory_applies_environment_and_restores_it() {
    let tree = tempfile::tempdir().unwrap();
    let mut session = open(
        r#"
        option build.dir {nested build}
        build.cmd printf
        build.args {'%s\n'} {"$PWD"} {"$KEEP"} {"$MACOSX_DEPLOYMENT_TARGET"} {"$SDKROOT"} {"$DEVELOPER_DIR"} {"$SOURCE_DATE_EPOCH"} {"$CPATH"} {"$LIBRARY_PATH"}
        build.env KEEP=override
        build.nice 19
    "#,
    );
    let output = executor(&mut session, tree.path().into());
    assert_eq!(
        session
            .invoke("command_exec", &["-notty", "build"])
            .unwrap(),
        ""
    );
    assert!(tree.path().join("nested build").is_dir());
    let text = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    for expected in [
        "override",
        "15.0",
        "/SDK path",
        "/Developer path",
        "12345",
        "/include/one:/include/two words",
        "/lib/one:/lib/two",
    ] {
        assert!(
            text.contains(&format!("-> building app: {expected}\n")),
            "{text}"
        );
    }
    assert_eq!(session.invoke("set", &["env(KEEP)"]).unwrap(), "original");
    assert_eq!(
        session
            .invoke("array", &["exists", "build.env_array"])
            .unwrap(),
        "0"
    );
    assert_eq!(
        session.invoke("info", &["exists", "env(SDKROOT)"]).unwrap(),
        "0"
    );
}

#[test]
fn varprefix_prebuilt_environment_shell_suffix_and_error_codes_are_preserved() {
    let tree = tempfile::tempdir().unwrap();
    let mut session = open(
        r#"
        commands custom
        option custom.dir created
        option build.dir .
        custom.env KEEP=ignored
        set custom.env_array(KEEP) prebuilt
        set compiler.log_verbose_output yes
        set workpath /work
        build.cmd printf
        build.args {'%s\n'} {"$KEEP"} {"$CC_PRINT_OPTIONS"} {"$CC_PRINT_OPTIONS_FILE"}
        proc failure {} {
            global errorCode
            set code [catch {command_exec -varprefix custom build {} {; exit 7}} message]
            return [list $code $errorCode]
        }
    "#,
    );
    let output = executor(&mut session, tree.path().into());
    let values = tcl_list::parse(&session.invoke("failure", &[]).unwrap()).unwrap();
    assert_eq!(values[0], "1");
    let code = tcl_list::parse(&values[1]).unwrap();
    assert_eq!(code[0], "CHILDSTATUS");
    assert_eq!(code[2], "7");
    assert!(tree.path().join("created").is_dir());
    assert_eq!(session.invoke("set", &["env(KEEP)"]).unwrap(), "original");
    assert_eq!(
        session
            .invoke("array", &["exists", "custom.env_array"])
            .unwrap(),
        "0"
    );
    assert_eq!(output.lock().unwrap().as_slice(), b"-> building app: prebuilt\n-> building app: YES\n-> building app: /work/.CC_PRINT_OPTIONS\n");
}

#[test]
fn directory_preparation_errors_restore_environment_without_launching_shell() {
    let tree = tempfile::tempdir().unwrap();
    std::fs::write(tree.path().join("file"), "occupied").unwrap();
    let mut session = open(
        r#"
        option build.dir file/child
        build.cmd printf
        build.args must-not-run
        build.env KEEP=changed
    "#,
    );
    let output = executor(&mut session, tree.path().into());
    assert!(session.invoke("command_exec", &["build"]).is_err());
    assert!(output.lock().unwrap().is_empty());
    assert_eq!(session.invoke("set", &["env(KEEP)"]).unwrap(), "original");
    assert_eq!(
        session
            .invoke("array", &["exists", "build.env_array"])
            .unwrap(),
        "0"
    );
}

#[test]
fn build_argument_helpers_match_make_and_ninja_parallelism_rules() {
    let mut session = open("set os.platform darwin; build.cmd /usr/bin/make; build.jobs 4");
    assert_eq!(
        session.invoke("portbuild::build_getargs", &[]).unwrap(),
        "-w all"
    );
    assert_eq!(
        session.invoke("portbuild::build_getjobsarg", &[]).unwrap(),
        " -j4"
    );
    session.invoke("build.jobs", &["1"]).unwrap();
    assert_eq!(
        session.invoke("portbuild::build_getjobsarg", &[]).unwrap(),
        ""
    );
    session.invoke("build.cmd", &["ninja"]).unwrap();
    assert_eq!(
        session.invoke("portbuild::build_getjobsarg", &[]).unwrap(),
        " -j1"
    );
    assert_eq!(
        session.invoke("portbuild::build_getargs", &[]).unwrap(),
        "all"
    );
    session.invoke("build.cmd", &["make"]).unwrap();
    session.invoke("set", &["os.platform", "freebsd"]).unwrap();
    assert_eq!(
        session.invoke("portbuild::build_getargs", &[]).unwrap(),
        "all"
    );
    session.invoke("build.type", &["gnu"]).unwrap();
    assert_eq!(
        session.invoke("portbuild::build_getargs", &[]).unwrap(),
        "-w all"
    );
    session.invoke("build.cmd", &["/usr/bin/clang"]).unwrap();
    assert_eq!(
        session.invoke("portbuild::build_getjobsarg", &[]).unwrap(),
        ""
    );
}

#[test]
fn native_build_failure_restores_command_options_and_environment() {
    let tree = tempfile::tempdir().unwrap();
    let mut session = open(
        r#"
        set os.platform darwin
        option build.dir build
        build.cmd false
        option build.jobs_arg { -j4}
        build.env KEEP=changed
        build {do-build}
        proc failed {} {global errorCode; catch {do-build}; return $errorCode}
    "#,
    );
    executor(&mut session, tree.path().into());
    let code = tcl_list::parse(&session.invoke("failed", &[]).unwrap()).unwrap();
    assert_eq!(code[0], "CHILDSTATUS");
    assert_eq!(code[2], "1");
    assert!(tree.path().join("build").is_dir());
    assert_eq!(session.scalar("build.cmd").unwrap(), "false");
    assert_eq!(
        session
            .invoke("info", &["exists", "build.pre_args"])
            .unwrap(),
        "0"
    );
    assert_eq!(session.invoke("set", &["env(KEEP)"]).unwrap(), "original");
}

#[cfg(target_os = "macos")]
#[test]
fn native_build_compiles_c_and_streams_compiler_diagnostics() {
    let tree = tempfile::tempdir().unwrap();
    std::fs::write(
        tree.path().join("hello.c"),
        "#pragma message(\"native build smoke\")\nint main(void) { return 0; }\n",
    )
    .unwrap();
    let mut session = open(
        r#"
        set os.platform darwin
        option build.dir build
        build.cmd /usr/bin/clang
        build.target {}
        build.args -o hello ../hello.c
        set macosx_deployment_target {}
        set configure.sdkroot {}
        set configure.developer_dir {}
        set compiler.cpath {}
        set compiler.library_path {}
        build {do-build}
    "#,
    );
    let output = executor(&mut session, tree.path().into());
    assert_eq!(session.invoke("do-build", &[]).unwrap(), "0");
    let text = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    assert!(text.contains("native build smoke"), "{text}");
    assert!(
        text.lines()
            .all(|line| line.starts_with("-> building app: ")),
        "{text}"
    );
    assert!(
        std::process::Command::new(tree.path().join("build/hello"))
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(session.scalar("build.cmd").unwrap(), "/usr/bin/clang");
}

#[test]
fn command_exec_uses_the_selected_variable_prefix_for_priority() {
    use std::os::unix::process::ExitStatusExt;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let mut session = open(
        r#"
        commands custom
        option custom.dir .
        option build.dir .
        build.cmd true
        build.nice 1
        custom.nice 8
    "#,
    );
    session
        .enable_system(&BTreeMap::new(), move |request| {
            captured
                .lock()
                .unwrap()
                .push((request.nice, request.new_session));
            Ok(build::ProcessResult {
                pid: 1,
                status: std::process::ExitStatus::from_raw(0),
            })
        })
        .unwrap();
    session
        .invoke("command_exec", &["-notty", "-varprefix", "custom", "build"])
        .unwrap();
    assert_eq!(*requests.lock().unwrap(), [(Some(8), true)]);
    session.invoke("custom.nice", &["invalid"]).unwrap();
    assert!(
        session
            .invoke("command_exec", &["-varprefix", "custom", "build"])
            .is_err()
    );
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[test]
fn command_execution_uses_native_build_environment_defaults() {
    let tree = tempfile::tempdir().unwrap();
    let mut session = Session::open(
        r#"
        name app; version 1
        set prefix /configured
        set workpath work
        set use_xcode no
        set macosx_deployment_target {}
        set configure.sdkroot {}
        set source_date_epoch 12345
        build.cmd /usr/bin/env
    "#,
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    let output = executor(&mut session, tree.path().into());
    session.invoke("command_exec", &["build"]).unwrap();
    let text = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    for expected in [
        "CPATH=/configured/include",
        "LIBRARY_PATH=/configured/lib",
        "CC_PRINT_OPTIONS=YES",
        "CC_PRINT_OPTIONS_FILE=work/.CC_PRINT_OPTIONS",
        "DEVELOPER_DIR=/Library/Developer/CommandLineTools",
        "SOURCE_DATE_EPOCH=12345",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    assert!(tree.path().join("work/app-1").is_dir());
}
