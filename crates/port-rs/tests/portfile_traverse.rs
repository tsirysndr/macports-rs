use port_rs::{
    portfile::{PortGroups, Session},
    spec::PortSpec,
    tcl_list,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::symlink,
    time::{Duration, UNIX_EPOCH},
};
fn session(source: &str) -> Session {
    Session::open(
        &format!("name app; version 1\n{source}"),
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap()
}
fn tree() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("a")).unwrap();
    fs::write(root.path().join("a/child"), "data").unwrap();
    fs::write(root.path().join("z"), "data").unwrap();
    symlink("a", root.path().join("link")).unwrap();
    symlink("missing", root.path().join("broken")).unwrap();
    root
}
#[test]
fn traversal_is_sorted_physical_and_supports_pre_and_post_order() {
    let root = tree();
    let mut session = session(
        r#"
        proc before {path} {
            set seen {}
            fs-traverse -tails item [list $path] {lappend seen $item}
            return $seen
        }
        proc after {path} {
            set seen {}
            fs-traverse -depth -tails item [list $path] {lappend seen $item}
            return $seen
        }
    "#,
    );
    let path = root.path().to_str().unwrap();
    assert_eq!(
        tcl_list::parse(&session.invoke("before", &[path]).unwrap()).unwrap(),
        [".", "a", "a/child", "broken", "link", "z"]
    );
    assert_eq!(
        tcl_list::parse(&session.invoke("after", &[path]).unwrap()).unwrap(),
        ["a/child", "a", "broken", "link", "z", "."]
    );
    let link = root.path().join("link");
    assert_eq!(
        tcl_list::parse(&session.invoke("before", &[link.to_str().unwrap()]).unwrap()).unwrap(),
        [".", "child"]
    );
}
#[test]
fn continue_prunes_directories_break_stops_and_return_leaves_the_procedure() {
    let root = tree();
    let mut session = session(
        r#"
        proc prune {path} {
            set seen {}
            fs-traverse -tails item [list $path] {
                lappend seen $item
                if {$item eq "a"} {continue}
                if {$item eq "link"} {break}
            }
            return $seen
        }
        proc early {path} {
            fs-traverse item [list $path] {return found}
            error unreachable
        }
        proc fails {path} {fs-traverse -ignoreErrors item [list $path] {error body-failed}}
    "#,
    );
    let path = root.path().to_str().unwrap();
    assert_eq!(
        tcl_list::parse(&session.invoke("prune", &[path]).unwrap()).unwrap(),
        [".", "a", "broken", "link"]
    );
    assert_eq!(session.invoke("early", &[path]).unwrap(), "found");
    assert!(
        session
            .invoke("fails", &[path])
            .unwrap_err()
            .to_string()
            .contains("body-failed")
    );
}
#[test]
fn traversal_validates_paths_and_supports_array_element_loop_variables() {
    let root = tree();
    let missing = root.path().join("absent");
    let mut session = session(
        r#"
        proc ignored {path} {
            fs-traverse -ignoreErrors item [list $path] {error should-not-run}
        }
        proc array_path {path} {
            fs-traverse -- item(path) [list $path] {break}
            return $item(path)
        }
    "#,
    );
    assert!(
        session
            .invoke("fs-traverse", &["item", missing.to_str().unwrap(), ""])
            .is_err()
    );
    assert_eq!(
        session
            .invoke("ignored", &[missing.to_str().unwrap()])
            .unwrap(),
        ""
    );
    assert_eq!(
        session
            .invoke("array_path", &[root.path().to_str().unwrap()])
            .unwrap(),
        root.path().to_str().unwrap()
    );
    assert!(
        session
            .invoke("fs-traverse", &["-tails", "item", "a b", ""])
            .unwrap_err()
            .to_string()
            .contains("multiple paths")
    );
    assert_eq!(
        session
            .invoke("fs-traverse", &["item", "", "error unexpected"])
            .unwrap(),
        ""
    );
}
#[test]
fn latest_timestamp_uses_regular_files_and_ignores_directory_and_link_times() {
    let root = tree();
    for (path, seconds) in [("a/child", 100), ("z", 200), ("a", 900), ("", 1000)] {
        fs::File::open(root.path().join(path))
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(seconds)))
            .unwrap();
    }
    let mut session = session("");
    let paths =
        molt::Value::from(vec![molt::Value::from(root.path().to_str().unwrap())]).to_string();
    assert_eq!(
        session
            .invoke("portutil::get_latest_path_mtime", &[&paths])
            .unwrap(),
        "200"
    );
    assert_eq!(
        session
            .invoke("portutil::get_latest_path_mtime", &[""])
            .unwrap(),
        "0"
    );
}
