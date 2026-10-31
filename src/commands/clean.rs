//! `port clean` (portclean): remove work directories, distfiles, temporary
//! archives and logs for ports in a local ports tree.
use crate::{index::Index, portfile::Session, query};
use anyhow::Result;
use clap::Args;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Args)]
pub struct Clean {
    /// Remove the work directory (the default)
    #[arg(long)]
    work: bool,
    /// Remove distfiles
    #[arg(long)]
    dist: bool,
    /// Remove temporary archives
    #[arg(long)]
    archive: bool,
    /// Remove logs
    #[arg(long)]
    logs: bool,
    /// Remove everything above
    #[arg(long)]
    all: bool,
    /// Remove a shared distfiles directory even if it holds other files
    #[arg(short = 'f', long = "force")]
    force: bool,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}

fn remove(path: &Path, progress: &crate::progress::Progress) {
    let result = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(_) => return,
    };
    if let Err(err) = result {
        progress.message(&format!("Error: cannot remove {}: {err}", path.display()));
    }
}

fn list(session: &mut Session, name: &str) -> Vec<String> {
    session
        .scalar(name)
        .ok()
        .and_then(|value| crate::tcl_list::parse(&value).ok())
        .unwrap_or_default()
}

pub(super) fn run(index: &Index, args: Clean, quiet: bool, root: &Path) -> Result<()> {
    let host = super::local::Host::new(root, None, quiet)?;
    let progress = host.progress.clone();
    let dist = args.all || args.dist;
    let archive = args.all || args.archive;
    let work = args.all || args.work || args.archive || args.dist || !args.logs;
    let keeplogs = host
        .context
        .variables
        .get("keeplogs")
        .is_some_and(|value| matches!(value.as_str(), "yes" | "1" | "true" | "on"));
    let logs = args.all || args.logs || !keeplogs;
    for mut spec in query::specifications(index, &args.ports)? {
        host.context.variants.apply(&mut spec);
        let port = query::lookup(index, &spec.name)?;
        let mut session = host.open(root, port, &spec)?;
        let subport = session.scalar("subport")?;
        progress.phase(&format!("Cleaning {subport}"));
        let portdbpath = PathBuf::from(session.scalar("portdbpath")?);
        if dist {
            let distpath = PathBuf::from(session.scalar("distpath")?);
            for file in list(&mut session, "distfiles")
                .into_iter()
                .chain(list(&mut session, "patchfiles"))
            {
                let name = session.invoke("getdistname", &[&file])?;
                let path = distpath.join(name);
                if path.is_file() {
                    remove(&path, &progress);
                }
            }
            let name = session.scalar("name")?;
            let subdir = session.scalar("dist_subdir")?;
            let mut directories = vec![name.clone()];
            if subdir != name {
                let shared = distpath.is_dir()
                    && fs::read_dir(&distpath).is_ok_and(|mut entries| entries.next().is_some());
                if shared && !args.force {
                    progress.warning(&format!(
                        "Distfiles directory '{}' may contain distfiles needed for other ports, use the -f flag to force removal",
                        distpath.display()
                    ));
                    directories.clear();
                } else {
                    directories.insert(0, subdir);
                }
            }
            for directory in directories {
                remove(&portdbpath.join("distfiles").join(directory), &progress);
            }
        }
        if archive {
            let incoming = portdbpath.join("incoming");
            for directory in [incoming.clone(), incoming.join("verified")] {
                let Ok(entries) = fs::read_dir(&directory) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let file = entry.file_name().to_string_lossy().into_owned();
                    let matches = file
                        .strip_prefix(&format!("{subport}-"))
                        .is_some_and(|rest| rest.contains('_') && rest.matches('.').count() >= 3);
                    if matches && entry.path().is_file() {
                        remove(&entry.path(), &progress);
                    }
                }
            }
        }
        if work {
            remove(Path::new(&session.scalar("subbuildpath")?), &progress);
            let link = PathBuf::from(session.scalar("worksymlink")?);
            if fs::symlink_metadata(&link).is_ok_and(|m| m.file_type().is_symlink()) {
                remove(&link, &progress);
            }
        }
        if logs {
            let portpath = session.scalar("portpath")?;
            let logpath = portdbpath.join("logs").join(portpath.replace('/', "_"));
            remove(&logpath.join(&subport), &progress);
            let _ = fs::remove_dir(&logpath);
        }
    }
    Ok(())
}
