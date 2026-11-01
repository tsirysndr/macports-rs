use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::collections::BTreeMap;

fn session(execute: bool) -> Session {
    let mut s = Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    if execute {
        s.enable_system(&BTreeMap::new(), |_| panic!("no shell expected"))
            .unwrap();
    }
    s
}

#[test]
fn channels_write_read_lines_and_track_eof() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("data.txt");
    let file = path.to_str().unwrap();
    let mut s = session(true);
    let script = format!(
        r#"
        set fd [open {file} w]
        puts $fd "first line"
        puts -nonewline $fd "second"
        close $fd
        set fd [open {file} a]
        puts $fd " line"
        close $fd
        set fd [open {file} r]
        set lines {{}}
        while {{[gets $fd line] >= 0}} {{ lappend lines $line }}
        set ended [eof $fd]
        seek $fd 0
        set head [read $fd 5]
        set where [tell $fd]
        close $fd
        list $lines $ended $head $where
    "#
    );
    assert_eq!(
        s.invoke("eval", &[&script]).unwrap(),
        "{{first line} {second line}} 1 first 5"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "first line\nsecond line\n"
    );
    let read_all =
        format!("set fd [open {file}]; set text [read -nonewline $fd]; close $fd; set text");
    assert_eq!(
        s.invoke("eval", &[&read_all]).unwrap(),
        "first line\nsecond line"
    );
    assert!(s.invoke("close", &["file999"]).is_err());
}

#[test]
fn evaluation_may_read_files_but_not_write_them() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("input.txt");
    std::fs::write(&path, "value\n").unwrap();
    let file = path.to_str().unwrap();
    let mut s = session(false);
    assert_eq!(
        s.invoke(
            "eval",
            &[&format!(
                "set fd [open {file}]; gets $fd v; close $fd; set v"
            )]
        )
        .unwrap(),
        "value"
    );
    assert!(
        s.invoke("open", &[&root.path().join("out").to_string_lossy(), "w"])
            .is_err()
    );
}

#[test]
fn open_reads_leading_zero_permissions_as_octal() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("wrapper");
    let file = path.to_str().unwrap();
    let mut s = session(true);
    s.invoke("eval", &[&format!("close [open {file} w 0755]")])
        .unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    let mask = unsafe {
        let mask = libc::umask(0);
        libc::umask(mask);
        mask as u32
    };
    assert_eq!(mode, 0o755 & !mask);
}
