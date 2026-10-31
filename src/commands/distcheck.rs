//! `port distcheck` (portdistcheck): check that mirrors still serve each
//! distfile, and whether it changed after the Portfile (moddate) or how big
//! it is (filesize).
use crate::{index::Index, install::source, query};
use anyhow::Result;
use std::{io::Write, path::Path, process::Command};

const SITES: &str = r#"
set fetch_urls {}
portfetch::checkfiles fetch_urls
set result {}
foreach {tag file} $fetch_urls {
    if {![info exists portfetch::urlmap($tag)]} {
        ui_error "No defined site for tag: $tag, using master_sites"
        set portfetch::urlmap($tag) $master_sites
    }
    lappend result $file $portfetch::urlmap($tag)
}
return $result
"#;

/// Headers of a HEAD request through curl(1), lowercased names.
fn head(url: &str, ignore_sslcert: bool) -> Result<Vec<(String, String)>, String> {
    let mut command = Command::new("/usr/bin/curl");
    command.args(["-s", "-S", "-f", "-I", "-L", "--max-time", "60"]);
    if ignore_sslcert {
        command.arg("-k");
    }
    let output = command.arg(url).output().map_err(|err| err.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect())
}

/// The last value of a header, following redirects to the final response.
fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .rev()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn human(total: u64) -> String {
    let size = total as f64;
    if total < 1024 {
        total.to_string()
    } else if total < 1024 * 1024 {
        format!("{:.1}K", size / 1024.0)
    } else if total < 1024 * 1024 * 1024 {
        format!("{:.1}M", size / (1024.0 * 1024.0))
    } else {
        format!("{:.1}G", size / (1024.0 * 1024.0 * 1024.0))
    }
}

pub(super) fn run(
    out: &mut impl Write,
    index: &Index,
    ports: &[String],
    quiet: bool,
    root: &Path,
) -> Result<()> {
    let host = super::local::Host::new(root, None, quiet)?;
    let progress = host.progress.clone();
    for mut spec in query::specifications(index, ports)? {
        host.context.variants.apply(&mut spec);
        let port = query::lookup(index, &spec.name)?;
        let mut session = host.open(root, port, &spec)?;
        let subport = session.scalar("subport")?;
        let kind = session
            .scalar("distcheck.type")
            .unwrap_or_else(|_| "moddate".into());
        let fetch_type = session.scalar("fetch.type").unwrap_or_default();
        if kind == "none" || fetch_type != "standard" {
            continue;
        }
        let ignore_sslcert = matches!(
            session
                .scalar("fetch.ignore_sslcert")
                .unwrap_or_default()
                .as_str(),
            "yes" | "1" | "true" | "on"
        );
        let portfile = root.join(port.get("portdir")).join("Portfile");
        let modified = std::fs::metadata(&portfile)?
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        let pairs = crate::tcl_list::parse(&session.invoke("uplevel", &["#0", SITES])?)?;
        let mut total = 0;
        for pair in pairs.chunks(2) {
            let distfile = &pair[0];
            let sites = crate::tcl_list::parse(pair.get(1).map(String::as_str).unwrap_or(""))?;
            let mut count = 0;
            match kind.as_str() {
                "moddate" => {
                    for site in &sites {
                        let url = source::assemble_url(site, distfile);
                        match head(&url, ignore_sslcert) {
                            Ok(headers) => {
                                count += 1;
                                let newer = header(&headers, "last-modified")
                                    .and_then(|value| httpdate::parse_http_date(value).ok())
                                    .and_then(|time| {
                                        time.duration_since(std::time::UNIX_EPOCH).ok()
                                    })
                                    .is_some_and(|time| time.as_secs() > modified);
                                if newer {
                                    progress.warning(&format!(
                                        "port {subport}: {url} is newer than Portfile"
                                    ));
                                }
                            }
                            Err(err) => progress.log(
                                "debug",
                                &format!("couldn't fetch {url} for {subport} ({err})"),
                            ),
                        }
                    }
                }
                "filesize" => {
                    for site in &sites {
                        let url = source::assemble_url(site, distfile);
                        match head(&url, ignore_sslcert) {
                            Ok(headers) => {
                                count += 1;
                                let size = header(&headers, "content-length")
                                    .and_then(|value| value.parse::<u64>().ok())
                                    .unwrap_or(0);
                                if size > 0 {
                                    progress.log(
                                        "info",
                                        &format!("port {subport}: {distfile} {size} bytes"),
                                    );
                                    total += size;
                                    break;
                                }
                            }
                            Err(err) => progress.log(
                                "debug",
                                &format!("couldn't fetch {url} for {subport} ({err})"),
                            ),
                        }
                    }
                }
                other => {
                    progress.error(&format!("unknown distcheck.type {other}"));
                    break;
                }
            }
            if count == 0 {
                progress.error(&format!("no mirror had {distfile} for {subport}"));
            }
        }
        if kind == "filesize" && total > 0 {
            writeln!(out, "{subport}: {}", human(total))?;
        }
    }
    Ok(())
}
