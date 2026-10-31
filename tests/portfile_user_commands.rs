use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, path::Path};

fn session(root: &Path) -> Session {
    let mut s = Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    s.enable_system(
        &BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
        |_| panic!("no system call expected"),
    )
    .unwrap();
    s.invoke("set", &["worksrcpath", root.to_str().unwrap()])
        .unwrap();
    s
}

fn eval(s: &mut Session, script: &str) -> String {
    s.invoke("eval", &[script]).unwrap()
}

#[test]
fn list_string_and_host_helpers() {
    let root = tempfile::tempdir().unwrap();
    let mut s = session(root.path());
    assert_eq!(eval(&mut s, "ldelete {a b c b} b"), "a c b");
    assert_eq!(
        eval(&mut s, "set l {}; lpush l a b; lunshift l z; set l"),
        "z a b"
    );
    assert_eq!(eval(&mut s, "lpop l"), "b");
    assert_eq!(eval(&mut s, "lshift l"), "z");
    assert_eq!(eval(&mut s, "set l"), "a");
    assert_eq!(eval(&mut s, "strsed foo-1.2 {s/-/_/g}"), "foo_1.2");
    assert_eq!(eval(&mut s, r"strsed abcabc {s/\(b\)c/[\1]/}"), "a[b]abc");
    // SAFETY: getuid only reads process credentials.
    let uid = unsafe { libc::getuid() };
    assert_eq!(eval(&mut s, "getuid"), uid.to_string());
    assert_eq!(eval(&mut s, "uid_to_name 0"), "root");
    assert_eq!(eval(&mut s, "name_to_gid wheel"), "0");
    assert!(eval(&mut s, "sysctl hw.ncpu").parse::<u32>().unwrap() > 0);
    fs::write(root.path().join("abc"), "abc").unwrap();
    assert_eq!(
        eval(
            &mut s,
            &format!("sha256 file {}/abc", root.path().display())
        ),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn exec_runs_pipelines_and_redirections() {
    let root = tempfile::tempdir().unwrap();
    let mut s = session(root.path());
    let out = root.path().join("out");
    assert_eq!(eval(&mut s, "exec echo hello"), "hello");
    assert_eq!(eval(&mut s, "exec printf {a\\nb\\n} | sort -r"), "b\na");
    assert_eq!(eval(&mut s, "exec -keepnewline echo x"), "x\n");
    assert_eq!(eval(&mut s, "exec cat << piped"), "piped");
    eval(&mut s, &format!("exec echo saved > {}", out.display()));
    assert_eq!(fs::read_to_string(&out).unwrap(), "saved\n");
    assert_eq!(
        eval(&mut s, "exec sh -c {echo err 1>&2; echo out} 2>@1"),
        "out\nerr"
    );
    assert_eq!(
        eval(&mut s, "exec -ignorestderr sh -c {echo err 1>&2; echo ok}"),
        "ok"
    );
    assert_eq!(
        eval(&mut s, "catch {exec sh -c {echo err 1>&2}} m; set m"),
        "err"
    );
    assert_eq!(eval(&mut s, "catch {exec false}"), "1");
    assert_eq!(
        eval(&mut s, "set env(MARK) seen; exec sh -c {echo $MARK}"),
        "seen"
    );
    assert!(s.invoke("exec", &["/nonexistent/tool"]).is_err());
}

#[test]
fn file_helpers_copy_move_link_touch_and_reinplace() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    let mut s = session(dir);
    fs::write(dir.join("a.txt"), "hello world\n").unwrap();
    fs::create_dir(dir.join("tree")).unwrap();
    fs::write(dir.join("tree/inner"), "inner").unwrap();
    let d = dir.display();
    eval(&mut s, &format!("copy {d}/a.txt {d}/b.txt"));
    assert_eq!(
        fs::read_to_string(dir.join("b.txt")).unwrap(),
        "hello world\n"
    );
    assert!(
        s.invoke("copy", &[&format!("{d}/a.txt"), &format!("{d}/b.txt")])
            .is_err()
    );
    eval(&mut s, &format!("file copy -force {d}/tree {d}/tree2"));
    assert_eq!(
        fs::read_to_string(dir.join("tree2/inner")).unwrap(),
        "inner"
    );
    eval(&mut s, &format!("move {d}/b.txt {d}/c.txt"));
    assert!(dir.join("c.txt").exists() && !dir.join("b.txt").exists());
    eval(&mut s, &format!("ln -s a.txt {d}/link"));
    assert_eq!(
        fs::read_link(dir.join("link")).unwrap().to_str(),
        Some("a.txt")
    );
    assert!(
        s.invoke("ln", &["-s", "a.txt", &format!("{d}/link")])
            .is_err()
    );
    eval(&mut s, &format!("ln -sf c.txt {d}/link"));
    assert_eq!(
        fs::read_link(dir.join("link")).unwrap().to_str(),
        Some("c.txt")
    );
    eval(&mut s, "touch new.txt");
    assert!(dir.join("new.txt").is_file());
    eval(&mut s, "touch -c absent.txt");
    assert!(!dir.join("absent.txt").exists());
    fs::set_permissions(dir.join("a.txt"), fs::Permissions::from_mode(0o444)).unwrap();
    eval(&mut s, "reinplace -q s/world/there/ a.txt");
    assert_eq!(
        fs::read_to_string(dir.join("a.txt")).unwrap(),
        "hello there\n"
    );
    assert_eq!(
        fs::metadata(dir.join("a.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o444
    );
    eval(&mut s, "reinplace -E {s/(hello) (there)/\\2 \\1/} a.txt");
    assert_eq!(
        fs::read_to_string(dir.join("a.txt")).unwrap(),
        "there hello\n"
    );
    assert!(s.invoke("reinplace", &["s/x/y/", "missing.txt"]).is_err());
    eval(&mut s, &format!("delete {d}/c.txt {d}/tree2"));
    assert!(!dir.join("c.txt").exists() && !dir.join("tree2").exists());
}

#[test]
fn xinstall_creates_directories_and_installs_files_with_modes() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    let mut s = session(dir);
    fs::write(dir.join("tool"), "#!/bin/sh\n").unwrap();
    fs::write(dir.join("README"), "doc").unwrap();
    let d = dir.display();
    eval(&mut s, &format!("xinstall -d -m 0750 {d}/dest/share/doc"));
    assert_eq!(
        fs::metadata(dir.join("dest/share/doc"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o750
    );
    eval(
        &mut s,
        &format!("xinstall -m 0644 -W {d} README {d}/dest/share/doc"),
    );
    assert_eq!(
        fs::read_to_string(dir.join("dest/share/doc/README")).unwrap(),
        "doc"
    );
    assert_eq!(
        fs::metadata(dir.join("dest/share/doc/README"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
    eval(&mut s, &format!("xinstall {d}/tool {d}/dest/bin-tool"));
    assert_eq!(
        fs::metadata(dir.join("dest/bin-tool"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    eval(
        &mut s,
        &format!("xinstall -m a=r,u+w {d}/tool {d}/dest/mode-tool"),
    );
    assert_eq!(
        fs::metadata(dir.join("dest/mode-tool"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
}

#[test]
fn mutations_require_an_execution_session() {
    let mut s = Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    for command in ["xinstall", "reinplace", "touch", "ln"] {
        assert!(s.invoke(command, &["x", "y"]).is_err(), "{command}");
    }
}
