use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{collections::BTreeMap, fs};

fn open_with(body: &str, groups: PortGroups) -> Session {
    Session::open(
        &format!("name app; version 1; {body}"),
        &BTreeMap::from([
            ("prefix".into(), "/p".into()),
            ("prefix_frozen".into(), "/p".into()),
            ("os.platform".into(), "darwin".into()),
            ("os.subplatform".into(), "macosx".into()),
            ("os.major".into(), "24".into()),
            ("build_arch".into(), "arm64".into()),
            ("developer_dir".into(), "/nonexistent/developer".into()),
        ]),
        &PortSpec::named("app"),
        groups,
    )
    .unwrap()
}

fn open(body: &str) -> Session {
    open_with(body, PortGroups::default())
}

#[test]
fn compiler_port_names_and_tool_paths_follow_upstream_naming() {
    let mut s = open("");
    for (compiler, port) in [
        ("macports-clang-17", "clang-17"),
        ("macports-clang-9.0", "clang-9.0"),
        ("macports-gcc-13", "gcc13"),
        ("macports-llvm-gcc-4.2", "llvm-gcc42"),
        ("apple-gcc-4.2", "apple-gcc42"),
        ("macports-mpich-clang-17", "mpich-clang17"),
        ("macports-openmpi-default", "openmpi-default"),
        ("macports-clang-devel", "clang-devel"),
        ("macports-g95", "g95"),
        ("clang", ""),
    ] {
        assert_eq!(
            s.invoke("portconfigure::compiler_port_name", &[compiler])
                .unwrap(),
            port,
            "{compiler}"
        );
    }
    for (kind, compiler, path) in [
        ("cc", "macports-clang-17", "/p/bin/clang-mp-17"),
        ("cxx", "macports-clang-17", "/p/bin/clang++-mp-17"),
        ("cpp", "macports-clang-17", "/p/bin/clang-cpp-mp-17"),
        ("cxx", "macports-clang-3.4", "/p/bin/clang++-mp-3.4"),
        ("fc", "macports-gcc-13", "/p/bin/gfortran-mp-13"),
        ("objcxx", "macports-gcc-13", "/p/bin/g++-mp-13"),
        (
            "cc",
            "macports-mpich-clang-17",
            "/p/bin/mpicc-mpich-clang17",
        ),
        (
            "f90",
            "macports-openmpi-gcc-12",
            "/p/bin/mpifort-openmpi-gcc12",
        ),
        ("cc", "macports-clang-devel", "/p/bin/clang-mp-devel"),
        ("cc", "apple-gcc-4.2", "/p/bin/gcc-apple-4.2"),
        ("fc", "macports-clang-17", ""),
        ("cc", "gcc-4.2", ""),
        ("cc", "llvm-gcc-4.2", ""),
    ] {
        assert_eq!(
            s.invoke("portconfigure::configure_get_compiler", &[kind, compiler])
                .unwrap(),
            path,
            "{kind} {compiler}"
        );
    }
}

#[test]
fn default_compiler_honours_blacklist_whitelist_and_reports_fallback() {
    let mut s = open("compiler.fallback bogus-a macports-clang-17\ncompiler.blacklist bogus-*");
    assert_eq!(s.scalar("configure.compiler").unwrap(), "macports-clang-17");
    assert_eq!(
        s.scalar("portconfigure::no_default_compiler_allowed")
            .unwrap(),
        "0"
    );
    s.invoke("compiler.whitelist", &["gcc-4.2"]).unwrap();
    assert_eq!(s.scalar("configure.compiler").unwrap(), "bogus-a");
    assert_eq!(
        s.scalar("portconfigure::no_default_compiler_allowed")
            .unwrap(),
        "1"
    );
    s.invoke("configure.compiler", &["macports-gcc-13"])
        .unwrap();
    assert_eq!(s.scalar("configure.cc").unwrap(), "/p/bin/gcc-mp-13");
    assert_eq!(s.scalar("configure.cxx").unwrap(), "/p/bin/g++-mp-13");
}

#[test]
fn fallback_sources_tree_compiler_lists_and_filters_by_standard() {
    let tree = tempfile::tempdir().unwrap();
    let compilers = tree.path().join("_resources/port1.0/compilers");
    fs::create_dir_all(&compilers).unwrap();
    fs::write(
        compilers.join("clang_compilers.tcl"),
        "lappend compilers macports-clang-30 macports-clang-2.9\n\
         if {${os.major} > 0} {return}\nlappend compilers never\n",
    )
    .unwrap();
    fs::write(
        compilers.join("gcc_compilers.tcl"),
        "lappend compilers macports-gcc-15\n",
    )
    .unwrap();
    let mut s = open_with(
        "set os.subplatform other\ncompiler.cxx_standard 2017",
        PortGroups::from_tree(tree.path()),
    );
    assert_eq!(
        s.scalar("compiler.fallback").unwrap(),
        "cc macports-clang-30"
    );
    s.invoke("set", &["cxx_stdlib", "libstdc++"]).unwrap();
    assert_eq!(
        s.scalar("configure.cxx_stdlib").unwrap(),
        "macports-libstdc++"
    );
    assert_eq!(
        s.scalar("compiler.fallback").unwrap(),
        "cc macports-clang-30 macports-gcc-15"
    );
    s.invoke("compiler.mpi", &["mpich"]).unwrap();
    assert_eq!(
        s.scalar("compiler.fallback").unwrap(),
        "macports-mpich-default macports-mpich-clang-30 macports-mpich-gcc-15"
    );
    s.invoke("set", &["default_compilers", "one two"]).unwrap();
    assert_eq!(s.scalar("compiler.fallback").unwrap(), "one two");
}

#[test]
fn stdlib_flags_and_architecture_options_follow_configuration() {
    let mut s = open("configure.compiler macports-clang-17");
    assert_eq!(s.scalar("configure.cxx_stdlib").unwrap(), "libc++");
    assert_eq!(
        s.scalar("configure.cxxflags").unwrap(),
        "-Os -stdlib=libc++"
    );
    assert_eq!(s.scalar("configure.cppflags").unwrap(), "-I/p/include");
    assert_eq!(
        s.scalar("configure.ldflags").unwrap(),
        "-L/p/lib -Wl,-headerpad_max_install_names"
    );
    s.invoke("set", &["configure.build_arch", "arm64"]).unwrap();
    assert_eq!(s.scalar("configure.cc_archflags").unwrap(), "-arch arm64");
    assert_eq!(s.scalar("configure.ld_archflags").unwrap(), "-arch arm64");
    s.invoke("configure.m32", &["yes"]).unwrap();
    assert_eq!(s.scalar("configure.cc_archflags").unwrap(), "-m32");
    s.invoke("set", &["configure.universal_archs", "arm64 x86_64"])
        .unwrap();
    assert_eq!(
        s.scalar("configure.universal_cflags").unwrap(),
        "-arch arm64 -arch x86_64"
    );
    s.invoke("set", &["compiler.cxx_standard", ""]).unwrap();
    assert_eq!(s.scalar("configure.cxx_stdlib").unwrap(), "");
    s.invoke("compiler.limit_flags", &["yes"]).unwrap();
    assert_eq!(s.scalar("configure.cppflags").unwrap(), "");
    s.invoke("set", &["system_options(macosx_sdk_path)", "/sdk"])
        .unwrap();
    assert_eq!(s.scalar("configure.sdkroot").unwrap(), "/sdk");
    assert_eq!(s.scalar("configure.sysroot").unwrap(), "/sdk");
}

#[test]
fn metadata_snapshot_does_not_resolve_execution_defaults() {
    let s = open("");
    let fields = &s.metadata().port;
    assert_eq!(fields.get("configure.compiler"), "");
    assert_eq!(fields.get("configure.sdkroot"), "");
    let s = open("configure.compiler clang");
    assert_eq!(s.metadata().port.get("configure.compiler"), "clang");
}
