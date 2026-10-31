//! Native verification of fetched source distfiles.
use super::*;
use crate::install::source::checksum::{self, Problem};
use molt::MoltResult;

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portchecksum::checksum_start", start, id);
    interp.add_context_command("portchecksum::checksum_main", main, id);
}

fn start(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    phase_start::notice(interp, id, argv, "Verifying checksums for")
}

fn main(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| verify(interp, id))?;
    Ok(0.into())
}

fn list(value: Value) -> Result<Vec<String>, Exception> {
    Ok(value.as_list()?.iter().map(Value::to_string).collect())
}

fn verify(interp: &mut Interp, id: ContextID) -> Result<(), Exception> {
    if !interp.var_exists(&"all_dist_files".into()) {
        return Ok(());
    }
    let progress = interp
        .context::<State>(id)
        .progress
        .clone()
        .unwrap_or_else(|| crate::progress::Progress::new(false));
    if interp.var_exists(&"checksum.skip".into()) && interp.scalar("checksum.skip")?.as_bool()? {
        let prefix = interp
            .scalar("UI_PREFIX")
            .map(|value| value.to_string())
            .unwrap_or_else(|_| "--->".into());
        progress.message(&format!("{prefix} Skipping checksum phase"));
        return Ok(());
    }
    let distfiles = list(interp.scalar("all_dist_files")?)?;
    let words = if interp.var_exists(&"checksums".into()) {
        list(interp.scalar("checksums")?)?
    } else {
        Vec::new()
    };
    let directory = interp.scalar("distpath")?.to_string();
    let directory = std::path::Path::new(&directory);
    let host = |err: anyhow::Error| error(format!("{err:#}"));
    let table = checksum::parse(&words, &distfiles);
    let problems = checksum::verify(directory, &distfiles, &table).map_err(host)?;
    if problems.is_empty() {
        return Ok(());
    }
    let mut moved = None;
    for problem in &problems {
        match problem {
            Problem::Missing(distfile) => {
                progress.message(&format!("Error: No checksum set for {distfile}"))
            }
            Problem::Mismatch {
                distfile,
                kind,
                expected,
                actual,
            } => {
                let kind = kind.name();
                progress.message(&format!("Error: Checksum ({kind}) mismatch for {distfile}"));
                progress.message(&format!("Portfile checksum: {distfile} {kind} {expected}"));
                progress.message(&format!("Distfile checksum: {distfile} {kind} {actual}"));
                let path = directory.join(distfile);
                if moved.is_none() && !is_html_name(distfile) && is_html(&path) {
                    let target = directory.join(format!("{distfile}.html"));
                    std::fs::rename(&path, &target).map_err(|err| host(err.into()))?;
                    moved = Some(target);
                }
            }
        }
    }
    if let Some(target) = moved {
        for line in [
            "***",
            "The non-matching file appears to be HTML. See this page for possible reasons",
            "for the checksum mismatch:",
            "<https://trac.macports.org/wiki/MisbehavingServers>",
            "***",
        ] {
            progress.message(line);
        }
        progress.message(&format!("The file has been moved to: {}", target.display()));
    } else {
        progress.message("The correct checksum line may be:");
        progress.message(&checksum::suggestion(directory, &distfiles, &table).map_err(host)?);
    }
    Err(error("Unable to verify file checksums"))
}

fn is_html_name(name: &str) -> bool {
    name.ends_with(".html") || name.ends_with(".htm")
}

fn is_html(path: &std::path::Path) -> bool {
    std::process::Command::new("/usr/bin/file")
        .args(["--brief", "--mime"])
        .arg(path)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .is_some_and(|output| {
            String::from_utf8_lossy(&output.stdout)
                .split(';')
                .next()
                .is_some_and(|kind| kind.trim() == "text/html")
        })
}

impl Session {
    /// Run the checksum target body. Every problem is reported before failing.
    pub fn verify_source_checksums(&mut self) -> Result<()> {
        check_supported(&mut self.interp, self.id)?;
        let id = self.id;
        let result = self
            .interp
            .with_global_scope(|interp| verify(interp, id))
            .map_err(|err| anyhow::anyhow!("{}", err.value()));
        check_supported(&mut self.interp, self.id)?;
        result
    }
}
