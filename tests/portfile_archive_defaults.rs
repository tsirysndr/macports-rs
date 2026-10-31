use port_rs::{
    portfile::{ArchiveSites, PortGroups, Session},
    spec::PortSpec,
};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

#[test]
fn append_and_prepend_resolve_live_archive_defaults_before_mutating() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let recorded = calls.clone();
    let archives = ArchiveSites::new(move |path, file| {
        recorded
            .borrow_mut()
            .push((path.to_owned(), file.to_owned()));
        Ok(vec!["official::tbz2".into(), "custom::tbz2".into()])
    });
    let mut session = Session::open_with_archive_sites(
        "name app; version 1; archive_sites.listpath alternate; archive_sites.listfile mirrors.tcl; archive_sites-append extra::tbz2; archive_sites-prepend first::tbz2",
        &BTreeMap::new(), &PortSpec::named("app"), PortGroups::default(), &BTreeMap::new(), archives,
    ).unwrap();
    assert_eq!(
        session.scalar("archive_sites").unwrap(),
        "first::tbz2 official::tbz2 custom::tbz2 extra::tbz2"
    );
    assert_eq!(
        *calls.borrow(),
        [("alternate".into(), "mirrors.tcl".into())]
    );
    session
        .invoke("archive_sites.listfile", &["later.tcl"])
        .unwrap();
    assert_eq!(
        session.scalar("archive_sites").unwrap(),
        "first::tbz2 official::tbz2 custom::tbz2 extra::tbz2"
    );
    assert_eq!(calls.borrow().len(), 1);
}

#[test]
fn untouched_defaults_track_resource_options_and_explicit_values_bypass_resolver() {
    let archives = ArchiveSites::new(|_, file| Ok(vec![format!("{file}::tbz2")]));
    let mut session = Session::open_with_archive_sites(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
        &BTreeMap::new(),
        archives,
    )
    .unwrap();
    assert_eq!(
        session.scalar("archive_sites").unwrap(),
        "archive_sites.tcl::tbz2"
    );
    session
        .invoke("archive_sites.listfile", &["new.tcl"])
        .unwrap();
    assert_eq!(session.scalar("archive_sites").unwrap(), "new.tcl::tbz2");
    for source in [
        "name app; version 1; archive_sites",
        "name app; version 1; archive_sites explicit::tbz2",
    ] {
        let archives = ArchiveSites::new(|_, _| anyhow::bail!("must not resolve explicit sites"));
        assert!(
            Session::open_with_archive_sites(
                source,
                &BTreeMap::new(),
                &PortSpec::named("app"),
                PortGroups::default(),
                &BTreeMap::new(),
                archives
            )
            .is_ok()
        );
    }
    let mut spec = PortSpec::named("app");
    spec.options
        .push(("archive_sites".into(), "command_line::tbz2".into()));
    let mut session = Session::open_with_archive_sites(
        "name app; version 1; archive_sites-append ignored::tbz2",
        &BTreeMap::new(),
        &spec,
        PortGroups::default(),
        &BTreeMap::new(),
        ArchiveSites::new(|_, _| anyhow::bail!("must not resolve CLI override")),
    )
    .unwrap();
    assert_eq!(
        session.scalar("archive_sites").unwrap(),
        "command_line::tbz2"
    );
}

#[test]
fn failed_defaults_are_reported_when_appending_and_imported_groups_share_resolver() {
    let result = Session::open_with_archive_sites(
        "name app; version 1; archive_sites-append extra::tbz2",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
        &BTreeMap::new(),
        ArchiveSites::new(|_, _| anyhow::bail!("bad resource")),
    );
    assert!(result.err().unwrap().to_string().contains("bad resource"));
    let mut groups = PortGroups::default();
    groups
        .insert("mirrors", "1.0", "archive_sites-append group::tbz2".into())
        .unwrap();
    let mut session = Session::open_with_archive_sites(
        "name app; version 1; PortGroup mirrors 1.0",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        groups,
        &BTreeMap::new(),
        ArchiveSites::new(|_, _| Ok(vec!["base::tbz2".into()])),
    )
    .unwrap();
    assert_eq!(
        session.scalar("archive_sites").unwrap(),
        "base::tbz2 group::tbz2"
    );
}
