//! `port distfiles` (portdistfiles): each distfile's path, checksums and URLs.
use crate::{index::Index, install::source, query};
use anyhow::Result;
use std::{io::Write, path::Path};

const SITES: &str = r#"
if {(![info exists master_sites] || $master_sites eq "{}")
    && (![info exists patchfiles] || ![info exists patch_sites] || $patch_sites eq "{}")} {
    return {}
}
set fetch_urls {}
portfetch::checkfiles fetch_urls
if {![info exists all_dist_files]} {
    return {}
}
set result {}
foreach {tag file} $fetch_urls {
    if {![info exists portfetch::urlmap($tag)]} {
        set portfetch::urlmap($tag) $portfetch::urlmap(master_sites)
    }
    lappend result $file $portfetch::urlmap($tag)
}
return $result
"#;

fn list(text: &str) -> Vec<String> {
    crate::tcl_list::parse(text).unwrap_or_default()
}

pub(super) fn run(
    out: &mut impl Write,
    index: &Index,
    ports: &[String],
    quiet: bool,
    root: &Path,
) -> Result<()> {
    let host = super::local::Host::new(root, None, quiet)?;
    for mut spec in query::specifications(index, ports)? {
        host.context.variants.apply(&mut spec);
        let port = query::lookup(index, &spec.name)?;
        let mut session = host.open(root, port, &spec)?;
        let subport = session.scalar("subport")?;
        writeln!(out, "--->  Distfiles for {subport}")?;
        let pairs = list(&session.invoke("uplevel", &["#0", SITES])?);
        if pairs.is_empty() {
            continue;
        }
        let distfiles: Vec<String> = pairs.iter().step_by(2).cloned().collect();
        let words = session
            .scalar("checksums")
            .map(|value| list(&value))
            .unwrap_or_default();
        let table = source::checksum::parse(&words, &distfiles);
        let directory = Path::new(&session.scalar("portdbpath")?)
            .join("distfiles")
            .join(session.scalar("dist_subdir")?);
        for pair in pairs.chunks(2) {
            let file = &pair[0];
            writeln!(out, "[{file}] {}", directory.join(file).display())?;
            for (kind, sum) in table.get(file).into_iter().flatten() {
                writeln!(out, " {}: {sum}", kind.name())?;
            }
            for site in list(pair.get(1).map(String::as_str).unwrap_or("")) {
                writeln!(out, "  {}", source::assemble_url(&site, file))?;
            }
            writeln!(out, " ")?;
        }
    }
    Ok(())
}
