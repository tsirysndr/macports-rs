use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::collections::BTreeMap;

fn open_with(body: &str, groups: PortGroups, spec: PortSpec) -> Session {
    Session::open(
        &format!("name app; version 1; {body}"),
        &BTreeMap::from([
            ("os.platform".into(), "darwin".into()),
            ("os.subplatform".into(), "macosx".into()),
            ("os.major".into(), "24".into()),
            ("build_arch".into(), "arm64".into()),
            ("universal_archs".into(), "arm64 x86_64".into()),
            ("configure.compiler".into(), "clang".into()),
        ]),
        &spec,
        groups,
    )
    .unwrap()
}

fn open(body: &str) -> Session {
    open_with(body, PortGroups::default(), PortSpec::named("app"))
}

#[test]
fn option_procs_trace_set_read_and_delete_without_recursing() {
    let mut s = open(
        r#"
        set events {}
        options my.option
        proc trace_option {option action args} {
            global events $option
            lappend events [list $action {*}$args]
            if {$action eq "read"} {set $option traced}
        }
        option_proc my.option trace_option
        my.option first
        set seen ${my.option}
        unset my.option
    "#,
    );
    assert_eq!(s.scalar("seen").unwrap(), "traced");
    assert_eq!(s.scalar("events").unwrap(), "{set first} read delete");
}

#[test]
fn callbacks_run_once_after_evaluation_and_variants() {
    let mut s = open(
        r#"
        set order {}
        variant extra {lappend order variant}
        default_variants +extra
        proc after_eval {} {global order; lappend order callback}
        port::register_callback after_eval
        lappend order portfile
    "#,
    );
    assert_eq!(s.scalar("order").unwrap(), "portfile variant callback");
    s.invoke("port::run_callbacks", &[]).unwrap();
    assert_eq!(s.scalar("order").unwrap(), "portfile variant callback");
}

#[test]
fn universal_variant_is_added_only_when_supported() {
    let s = open("");
    assert!(s.metadata().variants.contains_key("universal"));
    for body in [
        "universal_variant no",
        "use_configure no",
        "set os.universal_supported 0",
        "supported_archs arm64",
        "use_xmkmf yes",
    ] {
        let s = open(body);
        assert!(!s.metadata().variants.contains_key("universal"), "{body}");
    }
    let s = open("supported_archs arm64\nvariant universal {}");
    assert!(!s.metadata().variants.contains_key("universal"));
    let s = open("variant universal description custom {}");
    assert_eq!(s.metadata().variants["universal"].description, "custom");
}

#[test]
fn undescribed_variants_use_tree_descriptions() {
    let tree = tempfile::tempdir().unwrap();
    let conf = tree.path().join("_resources/port1.0");
    std::fs::create_dir_all(&conf).unwrap();
    std::fs::write(
        conf.join("variant_descriptions.conf"),
        "universal {Build for multiple architectures}\ndocs {Build documentation}\n",
    )
    .unwrap();
    let s = open_with(
        "variant docs {}\nvariant other {}",
        PortGroups::from_tree(tree.path()),
        PortSpec::named("app"),
    );
    let variants = &s.metadata().variants;
    assert_eq!(variants["docs"].description, "Build documentation");
    assert_eq!(
        variants["universal"].description,
        "Build for multiple architectures"
    );
    assert_eq!(variants["other"].description, "");
}
