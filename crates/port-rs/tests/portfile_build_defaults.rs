use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::collections::BTreeMap;
fn open(body: &str) -> Session {
    Session::open(
        &format!("name app; version 1\n{body}"),
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap()
}
fn enable(session: &mut Session) {
    session
        .enable_system(&BTreeMap::new(), |_| {
            panic!("reading defaults must not launch commands")
        })
        .unwrap();
}
#[test]
fn build_paths_and_compiler_environment_resolve_lazily_from_execution_inputs() {
    let mut session = open("");
    let snapshot = session.metadata().port.fields.clone();
    enable(&mut session);
    session.invoke("set", &["prefix", "/opt/local"]).unwrap();
    session
        .invoke("set", &["workpath", "/tmp/work path"])
        .unwrap();
    session
        .invoke("set", &["portpath", "/tmp/tree/app"])
        .unwrap();
    session.invoke("set", &["use_xcode", "no"]).unwrap();
    for (name, expected) in [
        ("worksrcdir", "app-1"),
        ("worksrcpath", "/tmp/work path/app-1"),
        ("build.dir", "/tmp/work path/app-1"),
        ("configure.dir", "/tmp/work path/app-1"),
        ("test.dir", "/tmp/work path/app-1"),
        ("destroot.dir", "/tmp/work path/app-1"),
        ("destroot", "/tmp/work path/destroot"),
        ("filespath", "/tmp/tree/app/files"),
        ("compiler.cpath", "/opt/local/include"),
        ("compiler.library_path", "/opt/local/lib"),
        ("compiler.log_verbose_output", "yes"),
        ("configure.cmd", "./configure"),
        (
            "configure.developer_dir",
            "/Library/Developer/CommandLineTools",
        ),
    ] {
        assert_eq!(session.scalar(name).unwrap(), expected, "{name}");
    }
    session.invoke("set", &["version", "2"]).unwrap();
    assert_eq!(session.scalar("build.dir").unwrap(), "/tmp/work path/app-2");
    session
        .invoke("set", &["build.cmd", "/usr/bin/make"])
        .unwrap();
    assert_eq!(session.scalar("test.cmd").unwrap(), "/usr/bin/make");
    assert_eq!(session.scalar("destroot.cmd").unwrap(), "/usr/bin/make");
    assert_eq!(session.metadata().port.fields, snapshot);
}
#[test]
fn explicit_and_lazy_portfile_overrides_survive_execution_initialization() {
    let mut session = open(
        r#"
        set selected /custom
        default compiler.cpath {${selected}/headers}
        compiler.library_path /custom/libraries
        compiler.log_verbose_output no
        option build.dir /custom/build
        set configure.developer_dir /custom/developer
    "#,
    );
    enable(&mut session);
    session.invoke("set", &["selected", "/changed"]).unwrap();
    assert_eq!(
        session.scalar("compiler.cpath").unwrap(),
        "/changed/headers"
    );
    assert_eq!(
        session.scalar("compiler.library_path").unwrap(),
        "/custom/libraries"
    );
    assert_eq!(session.scalar("compiler.log_verbose_output").unwrap(), "no");
    assert_eq!(session.scalar("build.dir").unwrap(), "/custom/build");
    assert_eq!(
        session.scalar("configure.developer_dir").unwrap(),
        "/custom/developer"
    );
    enable(&mut session);
    assert_eq!(
        session.scalar("compiler.cpath").unwrap(),
        "/changed/headers"
    );
}
#[test]
fn xcode_selection_uses_the_selected_developer_directory() {
    let mut session = open("build.type xcode");
    enable(&mut session);
    assert_eq!(session.scalar("use_xcode").unwrap(), "1");
    assert_eq!(
        session.scalar("configure.developer_dir").unwrap(),
        port_rs::source::toolchain::developer_dir(24)
    );
    session
        .invoke(
            "set",
            &[
                "developer_dir",
                "/Applications/Xcode.app/Contents/Developer",
            ],
        )
        .unwrap();
    assert_eq!(
        session.scalar("configure.developer_dir").unwrap(),
        "/Applications/Xcode.app/Contents/Developer"
    );
    session.invoke("set", &["use_xcode", "no"]).unwrap();
    assert_eq!(
        session.scalar("configure.developer_dir").unwrap(),
        "/Library/Developer/CommandLineTools"
    );
    assert!(
        session
            .invoke("portconfigure::configure_get_developer_dir", &["extra"])
            .is_err()
    );
}
