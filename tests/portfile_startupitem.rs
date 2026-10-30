use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};
fn session(root: &Path) -> Session {
    fs::create_dir(root.join("stage")).unwrap();
    fs::write(root.join("input.plist"), b"custom plist\0\xff").unwrap();
    let vars = BTreeMap::from([
        (
            "destroot".into(),
            root.join("stage").to_str().unwrap().into(),
        ),
        ("prefix".into(), "/opt/local".into()),
        ("startupitem.type".into(), "launchd".into()),
        (
            "startupitem.custom_file".into(),
            root.join("input.plist").to_str().unwrap().into(),
        ),
    ]);
    let mut s = Session::open(
        "name app; version 1",
        &vars,
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    s.enable_system(&BTreeMap::new(), |_| {
        panic!("staging must not run external commands")
    })
    .unwrap();
    s.initialize_creation_mask(0o022).unwrap();
    s
}
#[test]
fn stages_custom_plist_preserving_bytes_mode_and_time_with_absolute_link() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = session(temp.path());
    let source = temp.path().join("input.plist");
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(123456789);
    fs::File::open(&source)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(time))
        .unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o440)).unwrap();
    s.invoke("startupitem.install", &["yes"]).unwrap();
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    let base = temp.path().join("stage/opt/local/etc/LaunchDaemons");
    let staged = base.join("org.macports.app/org.macports.app.plist");
    assert_eq!(fs::read(&staged).unwrap(), b"custom plist\0\xff");
    assert_eq!(
        fs::metadata(&staged).unwrap().permissions().mode() & 0o7777,
        0o440
    );
    assert_eq!(fs::metadata(&staged).unwrap().modified().unwrap(), time);
    assert_eq!(
        fs::read_link(base.join("org.macports.app.plist"))
            .unwrap()
            .to_str()
            .unwrap(),
        "/opt/local/etc/LaunchDaemons/org.macports.app/org.macports.app.plist"
    );
    assert!(!temp.path().join("stage/Library").exists());
    assert!(
        s.invoke("portstartupitem::startupitem_create", &[])
            .is_err()
    );
    assert_eq!(fs::read(&staged).unwrap(), b"custom plist\0\xff");
}
#[test]
fn honors_agent_names_and_replaces_an_existing_link_without_following_it() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = session(temp.path());
    s.invoke("startupitem.location", &["LaunchAgents"]).unwrap();
    s.invoke("startupitem.name", &["worker"]).unwrap();
    s.invoke("startupitem.plist", &["worker.plist"]).unwrap();
    let base = temp.path().join("stage/opt/local/etc/LaunchAgents");
    fs::create_dir_all(&base).unwrap();
    let unrelated = temp.path().join("unrelated");
    fs::write(&unrelated, "preserve").unwrap();
    symlink(&unrelated, base.join("worker.plist")).unwrap();
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    assert_eq!(fs::read_to_string(unrelated).unwrap(), "preserve");
    assert_eq!(
        fs::read_link(base.join("worker.plist"))
            .unwrap()
            .to_str()
            .unwrap(),
        "/opt/local/etc/LaunchAgents/org.macports.worker/worker.plist"
    );
}
#[test]
fn rejects_escaping_names_and_linked_directories_without_outside_writes() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = session(temp.path());
    s.invoke("startupitem.uniquename", &["../escape"]).unwrap();
    assert!(
        s.invoke("portstartupitem::startupitem_create", &[])
            .is_err()
    );
    assert_eq!(fs::read_dir(temp.path().join("stage")).unwrap().count(), 0);
    s.invoke("startupitem.uniquename", &["org.macports.app"])
        .unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), temp.path().join("stage/opt")).unwrap();
    assert!(
        s.invoke("portstartupitem::startupitem_create", &[])
            .is_err()
    );
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}
#[test]
fn disabled_items_and_metadata_do_not_stage_files() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = session(temp.path());
    s.invoke("startupitem.type", &["none"]).unwrap();
    s.invoke("startupitem.create", &["yes"]).unwrap();
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    assert_eq!(fs::read_dir(temp.path().join("stage")).unwrap().count(), 0);
    let mut s = Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    assert!(
        s.invoke("portstartupitem::startupitem_create", &[])
            .is_err()
    );
}

#[test]
fn multiple_items_inherit_globals_and_keep_name_based_paths() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = session(temp.path());
    s.invoke("startupitem.uniquename", &["global-name"])
        .unwrap();
    s.invoke("startupitem.plist", &["global.plist"]).unwrap();
    s.invoke(
        "startupitems",
        &[
            "name",
            "first",
            "name",
            "second",
            "location",
            "LaunchAgents",
            "plist",
            "agent.plist",
            "name",
            "disabled",
            "type",
            "none",
        ],
    )
    .unwrap();
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    let root = temp.path().join("stage/opt/local/etc");
    assert_eq!(
        fs::read(root.join("LaunchDaemons/org.macports.first/org.macports.first.plist")).unwrap(),
        b"custom plist\0\xff"
    );
    assert_eq!(
        fs::read_link(root.join("LaunchAgents/agent.plist"))
            .unwrap()
            .to_str()
            .unwrap(),
        "/opt/local/etc/LaunchAgents/org.macports.second/agent.plist"
    );
    assert!(!root.join("LaunchDaemons/org.macports.disabled").exists());
    assert!(!root.join("LaunchDaemons/global-name").exists());
}

#[test]
fn repeated_item_name_merges_attributes_and_resets_derived_names() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = session(temp.path());
    s.invoke(
        "set",
        &[
            "startupitems",
            "name worker location LaunchAgents plist old.plist name worker",
        ],
    )
    .unwrap();
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    let base = temp.path().join("stage/opt/local/etc/LaunchAgents");
    assert!(
        base.join("org.macports.worker/org.macports.worker.plist")
            .is_file()
    );
    assert!(!base.join("org.macports.worker/old.plist").exists());
}

#[test]
fn invalid_later_item_is_rejected_before_any_item_is_written() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = session(temp.path());
    for specification in [
        "custom_file nowhere name worker",
        "name first name second custom_file /definitely/missing/port-rs.plist",
        "name worker plist ../escape",
    ] {
        s.invoke("set", &["startupitems", specification]).unwrap();
        assert!(
            s.invoke("portstartupitem::startupitem_create", &[])
                .is_err()
        );
        assert_eq!(fs::read_dir(temp.path().join("stage")).unwrap().count(), 0);
    }
}

#[test]
fn per_item_create_does_not_change_the_global_default_type() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = session(temp.path());
    s.invoke("unset", &["startupitem.type"]).unwrap();
    s.invoke("set", &["startupitems", "name worker create yes"])
        .unwrap();
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    assert_eq!(fs::read_dir(temp.path().join("stage")).unwrap().count(), 0);
    s.invoke("startupitem.create", &["yes"]).unwrap();
    s.invoke("set", &["system_options(startupitem_type)", "default"])
        .unwrap();
    s.invoke("set", &["os.platform", "darwin"]).unwrap();
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    assert!(
        temp.path()
            .join("stage/opt/local/etc/LaunchDaemons/org.macports.worker/org.macports.worker.plist")
            .is_file()
    );
}

fn generated_session(root: &Path) -> Session {
    let mut s = session(root);
    s.invoke("unset", &["startupitem.custom_file"]).unwrap();
    s.invoke("startupitem.create", &["yes"]).unwrap();
    s.invoke(
        "startupitem.executable",
        &["/opt/local/bin/server", "--title", "a<&\"b"],
    )
    .unwrap();
    s
}
#[cfg(target_os = "macos")]
fn plist_json(path: &Path) -> serde_json::Value {
    let output = std::process::Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
#[cfg(target_os = "macos")]
fn generates_valid_launchd_arguments_and_optional_fields() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = generated_session(temp.path());
    for (key, value) in [
        ("user", "service"),
        ("group", "staff"),
        ("logfile", "/tmp/out&log"),
        ("netchange", "yes"),
        ("logevents", "yes"),
        ("debug", "yes"),
    ] {
        s.invoke("set", &[&format!("startupitem.{key}"), value])
            .unwrap();
    }
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    let path = temp
        .path()
        .join("stage/opt/local/etc/LaunchDaemons/org.macports.app/org.macports.app.plist");
    let json = plist_json(&path);
    assert_eq!(
        json["ProgramArguments"],
        serde_json::json!([
            "/opt/local/bin/daemondo",
            "--label=app",
            "--start-cmd",
            "/opt/local/bin/server",
            "--title",
            "a<&\"b",
            ";",
            "--restart-netchange",
            "--verbosity=1",
            "--pid=exec"
        ])
    );
    assert_eq!(json["Disabled"], true);
    assert_eq!(json["KeepAlive"], true);
    assert_eq!(json["Debug"], true);
    assert_eq!(json["UserName"], "service");
    assert_eq!(json["GroupName"], "staff");
    assert_eq!(json["StandardOutPath"], "/tmp/out&log");
    assert_eq!(json["StandardErrorPath"], "/tmp/out&log");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o644
    );
    // Re-running generation replaces contents, but retains the existing mode.
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    s.invoke("startupitem.debug", &["no"]).unwrap();
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    assert!(plist_json(&path).get("Debug").is_none());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
}
#[test]
#[cfg(target_os = "macos")]
fn pidfile_modes_translate_and_default_run_directory_is_kept_once() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = generated_session(temp.path());
    let path = temp
        .path()
        .join("stage/opt/local/etc/LaunchDaemons/org.macports.app/org.macports.app.plist");
    for (style, flag) in [
        ("auto", "--pid=fileauto"),
        ("clean", "--pid=fileclean"),
        ("manual", "--pid=exec"),
        ("none", "--pid=none"),
    ] {
        s.invoke("startupitem.pidfile", &[style]).unwrap();
        s.invoke("portstartupitem::startupitem_create", &[])
            .unwrap();
        let json = plist_json(&path);
        let args = json["ProgramArguments"].as_array().unwrap();
        assert!(args.contains(&serde_json::json!(flag)));
        if style != "none" {
            assert_eq!(args.last().unwrap(), "/opt/local/var/run/app.pid");
        }
    }
    let keeps = s.scalar("destroot.keepdirs").unwrap();
    assert_eq!(s.invoke("llength", &[&keeps]).unwrap(), "1");
    s.invoke("startupitem.pidfile", &["manual", "/tmp/custom pid"])
        .unwrap();
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    assert_eq!(
        plist_json(&path)["ProgramArguments"]
            .as_array()
            .unwrap()
            .last()
            .unwrap(),
        "/tmp/custom pid"
    );
}
#[test]
fn invalid_xml_and_malformed_wrapper_lists_do_not_write_partial_plists() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = generated_session(temp.path());
    s.invoke(
        "startupitem.executable",
        &["/bin/server", "bad\u{1}argument"],
    )
    .unwrap();
    assert!(
        s.invoke("portstartupitem::startupitem_create", &[])
            .is_err()
    );
    assert_eq!(fs::read_dir(temp.path().join("stage")).unwrap().count(), 0);
    s.invoke("startupitem.executable", &["/bin/server"])
        .unwrap();
    s.invoke("set", &["startupitem.start", "{"]).unwrap();
    assert!(
        s.invoke("portstartupitem::startupitem_create", &[])
            .is_err()
    );
    assert_eq!(fs::read_dir(temp.path().join("stage")).unwrap().count(), 0);
}

#[test]
fn generated_wrapper_runs_init_and_selected_phases_and_default_restart() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = generated_session(temp.path());
    s.invoke("startupitem.init", &["echo init"]).unwrap();
    s.invoke("startupitem.start", &["echo start"]).unwrap();
    s.invoke("startupitem.stop", &["echo stop"]).unwrap();
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    let directory = temp
        .path()
        .join("stage/opt/local/etc/LaunchDaemons/org.macports.app");
    let wrapper = directory.join("app.wrapper");
    assert_eq!(
        fs::metadata(&wrapper).unwrap().permissions().mode() & 0o777,
        0o755
    );
    for (phase, expected) in [
        ("start", "init\nstart\n"),
        ("stop", "init\nstop\n"),
        ("restart", "init\nstop\nstart\n"),
    ] {
        let output = std::process::Command::new(&wrapper)
            .arg(phase)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
    }
    #[cfg(target_os = "macos")]
    {
        let json = plist_json(&directory.join("org.macports.app.plist"));
        let path = "/opt/local/etc/LaunchDaemons/org.macports.app/app.wrapper";
        assert_eq!(
            json["ProgramArguments"],
            serde_json::json!([
                "/opt/local/bin/daemondo",
                "--label=app",
                "--start-cmd",
                path,
                "start",
                ";",
                "--stop-cmd",
                path,
                "stop",
                ";",
                "--restart-cmd",
                path,
                "restart",
                ";",
                "--pid=exec"
            ])
        );
    }
    s.invoke("startupitem.restart", &["echo custom-restart"])
        .unwrap();
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    let output = std::process::Command::new(wrapper)
        .arg("restart")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "init\ncustom-restart\n"
    );
}

#[test]
fn default_wrapper_uses_subport_script_and_handles_quoted_prefix() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = generated_session(temp.path());
    s.invoke("unset", &["startupitem.executable"]).unwrap();
    let prefix = temp.path().join("prefix with ' quote");
    fs::create_dir_all(prefix.join("etc/startup")).unwrap();
    fs::write(prefix.join("etc/startup/app.sh"), "printf '%s\\n' \"$1\"\n").unwrap();
    s.invoke("set", &["prefix", prefix.to_str().unwrap()])
        .unwrap();
    s.invoke("portstartupitem::startupitem_create", &[])
        .unwrap();
    let directory = temp
        .path()
        .join("stage")
        .join(prefix.strip_prefix("/").unwrap())
        .join("etc/LaunchDaemons/org.macports.app");
    let output = std::process::Command::new(directory.join("app.wrapper"))
        .arg("restart")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "stop\nstart\n");
    #[cfg(target_os = "macos")]
    assert_eq!(
        plist_json(&directory.join("org.macports.app.plist"))["ProgramArguments"]
            .as_array()
            .unwrap()
            .last()
            .unwrap(),
        "--pid=none"
    );
}

#[test]
fn wrapper_links_and_colliding_filenames_cannot_overwrite_other_files() {
    let temp = tempfile::tempdir().unwrap();
    let mut s = generated_session(temp.path());
    s.invoke("startupitem.start", &["echo start"]).unwrap();
    s.invoke("startupitem.plist", &["app.wrapper"]).unwrap();
    assert!(
        s.invoke("portstartupitem::startupitem_create", &[])
            .is_err()
    );
    assert_eq!(fs::read_dir(temp.path().join("stage")).unwrap().count(), 0);
    s.invoke("startupitem.plist", &["app.plist"]).unwrap();
    let directory = temp
        .path()
        .join("stage/opt/local/etc/LaunchDaemons/org.macports.app");
    fs::create_dir_all(&directory).unwrap();
    let outside = temp.path().join("outside");
    fs::write(&outside, "preserve").unwrap();
    symlink(&outside, directory.join("app.wrapper")).unwrap();
    assert!(
        s.invoke("portstartupitem::startupitem_create", &[])
            .is_err()
    );
    assert_eq!(fs::read_to_string(outside).unwrap(), "preserve");
    assert!(!directory.join("app.plist").exists());
}
