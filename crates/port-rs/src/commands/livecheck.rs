//! `port livecheck` (portlivecheck): check whether upstream released a newer
//! version than the Portfile's.
use crate::{index::Index, portfile::Session, query};
use anyhow::{Context, Result, bail};
use std::{io::Write, path::Path, time::Duration};

/// Version extraction exactly as portlivecheck does it, run with `data`,
/// `the_re`, `current` and `multiline` set at global scope.
const MATCH: &str = r#"
set foundmatch 0
set updated_version 0
if {$multiline} {
    if {[regexp -nocase $the_re $data matched updated_version]} {
        set foundmatch 1
    }
} else {
    foreach line [split $data \n] {
        set lastoff 0
        while {$lastoff >= 0 && [regexp -nocase -start $lastoff -indices $the_re $line offsets]} {
            regexp -nocase -start $lastoff $the_re $line matched upver
            set foundmatch 1
            if {$updated_version == 0 || [vercmp $upver $updated_version] > 0} {
                set updated_version $upver
            }
            set lastoff [lindex $offsets end]
        }
    }
}
list $foundmatch $updated_version
"#;

/// Upstream's libcurl user agent; some servers (the GitHub API) format
/// responses differently for curl clients, which livecheck regexes rely on.
fn user_agent() -> String {
    let libcurl = std::process::Command::new("/usr/bin/curl")
        .arg("--version")
        .output()
        .ok()
        .and_then(|output| {
            String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .find(|word| word.starts_with("libcurl/"))
                .map(str::to_string)
        })
        .unwrap_or_else(|| "libcurl/8.7.1".into());
    format!(
        "MacPorts/{} {libcurl}",
        crate::portfile::MACPORTS_BASE_VERSION
    )
}

fn client(ignore_sslcert: bool) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(user_agent())
        .danger_accept_invalid_certs(ignore_sslcert)
        .timeout(Duration::from_secs(60))
        .build()?)
}

/// Fetch a URL; schemes other than HTTP(S), such as FTP, go through curl(1).
fn fetch(url: &str, ignore_sslcert: bool, headers: &[String]) -> Result<Vec<u8>> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        let mut command = std::process::Command::new("/usr/bin/curl");
        command.args(["-f", "-s", "-S", "-L"]);
        for header in headers {
            command.args(["-H", header]);
        }
        if ignore_sslcert {
            command.arg("-k");
        }
        let output = command.arg(url).output().context("cannot run curl")?;
        if !output.status.success() {
            let message = String::from_utf8_lossy(&output.stderr);
            let message = message.trim();
            let message = message
                .strip_prefix("curl: (")
                .and_then(|rest| rest.split_once(") "))
                .map(|(_, text)| text)
                .unwrap_or(message);
            bail!("{message}");
        }
        return Ok(output.stdout);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let mut request = client(ignore_sslcert)?.get(url);
        for header in headers {
            if let Some((name, value)) = header.split_once(':') {
                request = request.header(name.trim(), value.trim());
            }
        }
        let response = request.send().await?;
        if !response.status().is_success() {
            bail!(
                "The requested URL returned error: {}",
                response.status().as_u16()
            );
        }
        Ok(response.bytes().await?.to_vec())
    })
}

/// Whether the resource changed after `since` (seconds), by Last-Modified.
fn newer(url: &str, since: i64, ignore_sslcert: bool) -> Result<bool> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let modified = runtime.block_on(async {
        let response = client(ignore_sslcert)?
            .head(url)
            .send()
            .await?
            .error_for_status()?;
        anyhow::Ok(
            response
                .headers()
                .get(reqwest::header::LAST_MODIFIED)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string),
        )
    })?;
    let Some(modified) = modified else {
        return Ok(false);
    };
    let time = httpdate::parse_http_date(&modified).context("invalid Last-Modified")?;
    let seconds = time
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    Ok(seconds > since)
}

/// `Some(new version)` when updated, `None` when current; errors are reported.
fn check(
    session: &mut Session,
    portfile: &Path,
    progress: &crate::progress::Progress,
) -> Result<Option<(String, String)>> {
    let plan = session.livecheck_plan()?;
    let subport = plan.subport.clone();
    let failed = |progress: &crate::progress::Progress, error: String| {
        progress.error(&format!("cannot check if {subport} was updated ({error})"));
    };
    let mut updated_version = "unknown".to_string();
    let updated = match plan.kind.as_str() {
        "regex" | "regexm" => match fetch(&plan.url, plan.ignore_sslcert, &plan.headers) {
            Err(err) => {
                failed(progress, format!("{err:#}"));
                false
            }
            Ok(bytes) => {
                let data = String::from_utf8_lossy(&bytes).into_owned();
                let multiline = if plan.kind == "regexm" { "1" } else { "0" };
                for (name, value) in [
                    ("data", data.as_str()),
                    ("the_re", plan.regex.as_str()),
                    ("multiline", multiline),
                ] {
                    session.invoke("set", &[&format!("::port_rs_livecheck_{name}"), value])?;
                }
                let script = MATCH
                    .replace("$data", "$::port_rs_livecheck_data")
                    .replace("$the_re", "$::port_rs_livecheck_the_re")
                    .replace("$multiline", "$::port_rs_livecheck_multiline");
                let result = session.invoke("uplevel", &["#0", &script])?;
                let fields = crate::tcl_list::parse(&result)?;
                if fields.first().map(String::as_str) != Some("1") {
                    progress.error(&format!(
                        "cannot check if {subport} was updated (regex didn't match)"
                    ));
                    false
                } else {
                    updated_version = fields.get(1).cloned().unwrap_or_default();
                    if updated_version == plan.version {
                        false
                    } else if session
                        .invoke("vercmp", &[&updated_version, &plan.version])?
                        .parse::<i64>()
                        .unwrap_or(0)
                        > 0
                    {
                        true
                    } else {
                        progress.error(&format!(
                            "livecheck failed for {subport}: extracted version '{updated_version}' is older than livecheck.version '{}'",
                            plan.version
                        ));
                        false
                    }
                }
            }
        },
        "md5" => match fetch(&plan.url, plan.ignore_sslcert, &plan.headers) {
            Err(err) => {
                failed(progress, format!("{err:#}"));
                false
            }
            Ok(bytes) => {
                use md5::Digest;
                let sum: String = md5::Md5::digest(&bytes)
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                sum != plan.md5
            }
        },
        "moddate" => {
            let modified = std::fs::metadata(portfile)?
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or(0);
            match newer(&plan.url, modified, plan.ignore_sslcert) {
                Ok(updated) => updated,
                Err(err) => {
                    failed(progress, format!("{err:#}"));
                    false
                }
            }
        }
        "git" => {
            let branch = if plan.branch.is_empty() {
                "HEAD"
            } else {
                &plan.branch
            };
            let output = std::process::Command::new("git")
                .args(["ls-remote", &plan.url, branch])
                .output();
            match output {
                Ok(output) if output.status.success() => {
                    updated_version = String::from_utf8_lossy(&output.stdout)
                        .split_whitespace()
                        .next()
                        .unwrap_or_default()
                        .to_string();
                    updated_version != plan.version
                }
                Ok(output) => {
                    failed(
                        progress,
                        String::from_utf8_lossy(&output.stderr).trim().to_string(),
                    );
                    false
                }
                Err(err) => {
                    failed(progress, err.to_string());
                    false
                }
            }
        }
        "none" => return Ok(None),
        other => {
            progress.error(&format!("unknown livecheck.type {other}"));
            return Ok(None);
        }
    };
    if updated {
        Ok(Some((plan.version, updated_version)))
    } else {
        progress.log("info", &format!("{subport} seems to be up to date"));
        Ok(None)
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
    for mut spec in query::specifications(index, ports)? {
        host.context.variants.apply(&mut spec);
        let port = query::lookup(index, &spec.name)?;
        let mut session = host.open(root, port, &spec)?;
        let portfile = root.join(port.get("portdir")).join("Portfile");
        match check(&mut session, &portfile, &host.progress) {
            Ok(Some((current, new))) => {
                let subport = session.scalar("subport")?;
                writeln!(
                    out,
                    "{subport} seems to have been updated (port version: {current}, new version: {new})"
                )?;
            }
            Ok(None) => {}
            Err(err) => bail!("{}: livecheck: {err:#}", spec.name),
        }
    }
    Ok(())
}
