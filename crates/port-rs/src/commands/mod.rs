//! CLI dispatch. Each action owns its options and presentation in a separate module.
mod archivefetch;
mod base_update;
mod bump;
mod clean;
mod contents;
mod dependencies;
mod dependents;
mod diagnose;
mod distcheck;
mod distfiles;
mod download;
mod help;
mod info;
mod inspect;
mod install;
mod install_exec;
mod installed;
mod launchd;
mod lifecycle;
mod lint;
mod livecheck;
mod local;
mod location;
mod mirror;
mod outdated;
mod package;
mod phase;
mod portcmds;
pub mod portindex;
mod provides;
mod pseudo;
mod reclaim;
mod restore;
mod rev_upgrade;
mod sandbox;
mod search;
mod select;
mod size;
mod snapshot;
mod space;
mod sync;
mod system;
mod trace;
mod upgrade;
mod variants;

use anyhow::{Context, Result, bail};

/// Every upstream action, plus this implementation's extensions.
pub const ACTIONS: &[&str] = &[
    "usage",
    "help",
    "echo",
    "info",
    "location",
    "notes",
    "provides",
    "log",
    "activate",
    "deactivate",
    "select",
    "sync",
    "selfupdate",
    "setrequested",
    "unsetrequested",
    "setunrequested",
    "clean",
    "upgrade",
    "rev-upgrade",
    "reclaim",
    "diagnose",
    "version",
    "platform",
    "prefix",
    "uninstall",
    "mirror",
    "installed",
    "outdated",
    "contents",
    "space",
    "dependents",
    "rdependents",
    "deps",
    "rdeps",
    "variants",
    "search",
    "list",
    "edit",
    "cat",
    "dir",
    "work",
    "cd",
    "url",
    "file",
    "logfile",
    "gohome",
    "fetch",
    "checksum",
    "extract",
    "patch",
    "configure",
    "build",
    "destroot",
    "install",
    "test",
    "lint",
    "livecheck",
    "distcheck",
    "bump",
    "load",
    "unload",
    "reload",
    "distfiles",
    "archivefetch",
    "archive",
    "unarchive",
    "dmg",
    "mdmg",
    "mpkg",
    "pkg",
    "snapshot",
    "restore",
    "migrate",
    "quit",
    "exit",
    "download", // Rust extension.
];

use clap::Subcommand;
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Subcommand)]
pub enum Command {
    /// Preview a binary installation plan with -y (installation not yet implemented)
    Install(install::Install),
    /// Show the recorded location of an installed port image
    Location(location::Location),
    /// Report disk space used by active ports
    Space(space::Space),
    /// Compare active installations with the available catalog
    Outdated(outdated::Outdated),
    /// Show installed ports that depend on a port
    Dependents(dependents::Dependents),
    /// Show recursive installed-port dependents
    Rdependents(dependents::RecursiveDependents),
    /// Find the active port that owns a filesystem path
    Provides(provides::Provides),
    /// List files recorded for an installed port (read-only)
    Contents(contents::Contents),
    /// List ports recorded in the installed-port registry (read-only)
    Installed(installed::Installed),
    /// Display the version of this standalone Rust implementation
    Version,
    /// Display the native MacPorts platform and architecture
    Platform,
    /// List ports from a local MacPorts PortIndex
    List {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        ports: Vec<String>,
    },
    /// Search names and descriptions (case insensitive by default)
    Search(search::Search),
    /// Display indexed port metadata
    Info(info::Info),
    /// Print selected port specifications
    Echo(inspect::Selection),
    /// Display dependencies from local Portfiles, or cached metadata with --index
    Deps(dependencies::DependencyOptions),
    /// Display recursive dependencies from local Portfiles or --index
    Rdeps(dependencies::RecursiveDependencyOptions),
    /// Display variants from local Portfiles, or cached metadata with --index
    Variants(variants::Variants),
    /// Download files concurrently from a checksummed JSON manifest (Rust extension)
    Download(download::Download),
    /// Print a port's Portfile
    Cat(portcmds::Ports),
    /// Edit a port's Portfile with $MP_EDITOR, $VISUAL or $EDITOR
    Edit(portcmds::Edit),
    /// Print the path to a port's directory
    Dir(portcmds::Ports),
    /// Print the path to a port's work directory, if it exists
    Work(portcmds::Ports),
    /// Print the file URL of a port's directory
    Url(portcmds::Ports),
    /// Print the path to a port's Portfile
    File(portcmds::Ports),
    /// Print the path to a port's main build log
    Logfile(portcmds::Ports),
    /// Show a port's build log, optionally filtered by phase and level
    Log(portcmds::Log),
    /// List a port's distfiles with their checksums and download URLs
    Distfiles(portcmds::Ports),
    /// Show the command syntax, or the usage of the given actions
    Usage { topics: Vec<String> },
    /// Show the manual page for port or one of its actions
    Help { topics: Vec<String> },
    /// Update a Portfile's checksums to match freshly fetched distfiles
    Bump(bump::Bump),
    /// Check the system for common problems
    Diagnose(diagnose::Diagnose),
    /// Fetch and verify distfiles for mirroring
    Mirror(mirror::Mirror),
    /// Check that mirrors still serve each port's distfiles
    Distcheck(portcmds::Ports),
    /// Check whether upstream released newer versions of ports
    Livecheck(portcmds::Ports),
    /// Check Portfiles against MacPorts conventions
    Lint(lint::Lint),
    /// Record, list, compare, export or import registry snapshots
    Snapshot(snapshot::SnapshotArgs),
    /// Reinstall the ports recorded in a snapshot
    Restore(restore::Restore),
    /// Snapshot, uninstall and reinstall all ports for a new OS or architecture
    Migrate(restore::Migrate),
    /// Rebuild ports whose binaries have broken library linkage
    #[command(name = "rev-upgrade")]
    RevUpgrade(rev_upgrade::RevUpgrade),
    /// Uninstall unneeded and inactive ports and delete unused files
    Reclaim(reclaim::Reclaim),
    /// Build a macOS installer package for ports
    Pkg(portcmds::Ports),
    /// Build a macOS installer package including runtime dependencies
    Mpkg(portcmds::Ports),
    /// Build a disk image containing a port's installer package
    Dmg(portcmds::Ports),
    /// Build a disk image containing a port's meta-package
    Mdmg(portcmds::Ports),
    /// Unpack ports' binary archives into their destroot
    Unarchive(portcmds::Ports),
    /// Fetch and verify ports' signed binary archives
    Archivefetch(portcmds::Ports),
    /// Build ports and store their images without activating them
    Archive(portcmds::Ports),
    /// Load a port's startup items with launchctl
    Load(portcmds::Ports),
    /// Unload a port's startup items with launchctl
    Unload(portcmds::Ports),
    /// Unload and load a port's startup items again
    Reload(portcmds::Ports),
    /// Open a port's homepage in the default browser
    Gohome(portcmds::Ports),
    /// Show a port's notes
    Notes(portcmds::Ports),
    /// Mark installed ports as requested
    Setrequested(portcmds::Ports),
    /// Mark installed ports as unrequested
    Unsetrequested(portcmds::Ports),
    /// Mark installed ports as unrequested
    Setunrequested(portcmds::Ports),
    /// Rebuild and activate newer versions of outdated installed ports
    Upgrade(upgrade::Upgrade),
    /// Activate an installed, inactive port image
    Activate(lifecycle::Lifecycle),
    /// Deactivate an active port image, removing its files from the prefix
    Deactivate(lifecycle::Lifecycle),
    /// Uninstall port images, deactivating them first if necessary
    Uninstall(lifecycle::Uninstall),
    /// Fetch distfiles for ports in a local ports tree
    Fetch(phase::Fetch),
    /// Fetch and verify distfile checksums for ports in a local ports tree
    Checksum(phase::Phase),
    /// Extract distfiles for ports in a local ports tree
    Extract(phase::Phase),
    /// Apply patches for ports in a local ports tree
    Patch(phase::Phase),
    /// Configure ports in a local ports tree
    Configure(phase::Phase),
    /// Build ports in a local ports tree
    Build(phase::Phase),
    /// Stage ports into their destroot in a local ports tree
    Destroot(phase::Phase),
    /// Build and run a port's test suite in a local ports tree
    Test(phase::Phase),
    /// Remove work directories, distfiles, archives or logs
    Clean(clean::Clean),
    /// Update the ports trees listed in sources.conf
    Sync,
    /// Choose among alternative versions installed for a select group
    Select(select::Select),
    /// Update the ports trees; port-rs itself is not replaced
    Selfupdate {
        /// Skip updating the ports trees
        #[arg(long, visible_alias = "no-sync")]
        nosync: bool,
        /// Reinstall port-rs even if it is already the latest version
        #[arg(long)]
        migrate: bool,
        /// Accepted for compatibility; port-rs downloads release binaries
        #[arg(long)]
        rsync: bool,
    },
    #[command(external_subcommand)]
    Unsupported(Vec<String>),
}

/// Actions that modify the system and have no dry-run of their own.
fn changes_state(command: &Command) -> Option<&'static str> {
    Some(match command {
        Command::Upgrade(_) => "upgrade",
        Command::Activate(_) => "activate",
        Command::Deactivate(_) => "deactivate",
        Command::Uninstall(_) => "uninstall",
        Command::Setrequested(_) => "setrequested",
        Command::Unsetrequested(_) | Command::Setunrequested(_) => "setunrequested",
        Command::Fetch(_) => "fetch",
        Command::Checksum(_) => "checksum",
        Command::Extract(_) => "extract",
        Command::Patch(_) => "patch",
        Command::Configure(_) => "configure",
        Command::Build(_) => "build",
        Command::Destroot(_) => "destroot",
        Command::Test(_) => "test",
        Command::Clean(_) => "clean",
        Command::Sync => "sync",
        Command::Selfupdate { .. } => "selfupdate",
        Command::Select(_) => "select",
        Command::Snapshot(_) => "snapshot",
        Command::Restore(_) => "restore",
        Command::Migrate(_) => "migrate",
        Command::Load(_) => "load",
        Command::Unload(_) => "unload",
        Command::Reload(_) => "reload",
        Command::Archive(_) => "archive",
        Command::Archivefetch(_) => "archivefetch",
        Command::Unarchive(_) => "unarchive",
        Command::Pkg(_) => "pkg",
        Command::Mpkg(_) => "mpkg",
        Command::Dmg(_) => "dmg",
        Command::Mdmg(_) => "mdmg",
        Command::Mirror(_) => "mirror",
        Command::Bump(_) => "bump",
        _ => return None,
    })
}

/// The directory for `-D` and the shell's `cd`: a path as given, or a port's
/// directory in the selected ports tree.
pub fn port_directory(target: &str, index_file: Option<&Path>) -> Result<PathBuf> {
    if target.is_empty() {
        bail!("cd requires a port name or directory");
    }
    if target.contains('/') || target == "." || target == ".." {
        return Ok(PathBuf::from(target));
    }
    let default_index = (index_file.is_none() && !Path::new("PortIndex").exists())
        .then(sync::default_index)
        .flatten();
    let index_file = index_file.or(default_index.as_deref());
    let index = crate::source::load(index_file, true)?;
    let port = crate::query::lookup(&index, target)
        .map_err(|_| anyhow::anyhow!("port -D failed to look up {target}: no such port"))?;
    let root = index_file
        .and_then(Path::parent)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(root.join(port.get("portdir")))
}

/// Where installs may take ports from (upstream `-s` and `-b`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveMode {
    Any,
    SourceOnly,
    BinaryOnly,
}

static ARCHIVE_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub fn set_archive_mode(mode: ArchiveMode) {
    ARCHIVE_MODE.store(mode as u8, std::sync::atomic::Ordering::Relaxed);
}

/// Port names in the selected PortIndex (explicit, `./PortIndex`, or the
/// default source's), for shell completion.
pub fn port_names(index: Option<&Path>) -> Vec<String> {
    let path = index
        .map(Path::to_path_buf)
        .or_else(|| Some(PathBuf::from("PortIndex")).filter(|path| path.is_file()))
        .or_else(sync::default_index);
    let Some(text) = path.and_then(|path| std::fs::read(path).ok()) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    let mut offset = 0;
    while let Some(end) = text[offset..].iter().position(|byte| *byte == b'\n') {
        let header = String::from_utf8_lossy(&text[offset..offset + end]).into_owned();
        let mut words = header.split_whitespace();
        let (Some(name), Some(Ok(length))) = (words.next(), words.next().map(str::parse::<usize>))
        else {
            break;
        };
        names.push(name.to_string());
        offset += end + 1 + length;
        if offset >= text.len() {
            break;
        }
    }
    names
}

/// Ask a yes/no question on a terminal; non-interactive runs take the
/// default answer, as upstream.
pub(crate) fn ask(question: &str, default: bool) -> bool {
    use std::io::{BufRead, IsTerminal};
    if crate::options::get().noninteractive || !io::stdin().is_terminal() {
        return default;
    }
    eprint!("{question} [{}]: ", if default { "Y/n" } else { "y/N" });
    let _ = io::stderr().flush();
    let mut answer = String::new();
    if io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    match answer.trim() {
        "" => default,
        answer => matches!(answer, "y" | "Y" | "yes" | "Yes"),
    }
}

pub(crate) fn archive_mode() -> ArchiveMode {
    match ARCHIVE_MODE.load(std::sync::atomic::Ordering::Relaxed) {
        1 => ArchiveMode::SourceOnly,
        2 => ArchiveMode::BinaryOnly,
        _ => ArchiveMode::Any,
    }
}

pub fn run(command: Command, index_file: Option<&Path>, quiet: bool, verbose: bool) -> Result<()> {
    Session::default().run(command, index_file, None, quiet, verbose)
}

/// A command chain shares its parsed catalog until the selected source changes.
#[derive(Default)]
pub struct Session {
    catalog: Option<(Option<PathBuf>, crate::index::Index)>,
}
impl Session {
    fn index(&mut self, path: Option<&Path>, quiet: bool) -> Result<&crate::index::Index> {
        if !self
            .catalog
            .as_ref()
            .is_some_and(|(cached, _)| cached.as_deref() == path)
        {
            self.catalog = Some((
                path.map(Path::to_path_buf),
                crate::source::load(path, quiet)?,
            ));
        }
        Ok(&self.catalog.as_ref().unwrap().1)
    }
    pub fn run(
        &mut self,
        command: Command,
        index_file: Option<&Path>,
        registry_file: Option<&Path>,
        quiet: bool,
        verbose: bool,
    ) -> Result<()> {
        if let Command::Unsupported(args) = &command {
            bail!(
                "command {:?} is not implemented in the standalone Rust engine; no Tcl or MacPorts fallback is used",
                args[0]
            );
        }
        if crate::options::get().dry_run
            && let Some(action) = changes_state(&command)
        {
            eprintln!("--->  Skipping {action} (dry run)");
            return Ok(());
        }

        match command {
            Command::Select(args) => {
                let stdout = io::stdout();
                let mut out = io::BufWriter::new(stdout.lock());
                select::run(&mut out, args, quiet)?;
                out.flush()?;
                return Ok(());
            }
            Command::Usage { topics } => return help::usage(&topics),
            Command::Help { topics } => {
                return help::help(&topics, |action| {
                    use clap::Subcommand;
                    let mut command = Command::augment_subcommands(
                        clap::Command::new("port")
                            .about("Standalone Rust implementation of the MacPorts port CLI")
                            .styles(crate::theme::help_styles()),
                    );
                    let help = match action.and_then(|action| command.find_subcommand_mut(action)) {
                        Some(subcommand) => {
                            let name = subcommand.get_name().to_string();
                            subcommand
                                .clone()
                                .bin_name(format!("port {name}"))
                                .render_long_help()
                        }
                        None => command.render_long_help(),
                    };
                    println!("{}", help.ansi());
                    Ok(())
                });
            }
            Command::Sync => {
                self.catalog = None;
                return sync::run(quiet, false);
            }
            Command::Selfupdate {
                nosync, migrate, ..
            } => {
                self.catalog = None;
                let reinstall = migrate || crate::options::get().force;
                base_update::update(&crate::progress::Progress::new(quiet), reinstall)?;
                return if nosync {
                    Ok(())
                } else {
                    sync::run(quiet, true)
                };
            }
            _ => {}
        }
        let default_index = (index_file.is_none() && !Path::new("PortIndex").exists())
            .then(sync::default_index)
            .flatten();
        let index_file = index_file.or(default_index.as_deref());
        let configured_path = registry_file
            .is_none()
            .then(local::configured_registry)
            .flatten();
        let configured = configured_path.clone().filter(|path| path.exists());
        // Commands that create or update the registry use the configured
        // location even before it exists.
        let writable_registry = registry_file
            .map(Path::to_path_buf)
            .or(configured_path)
            .unwrap_or_else(|| PathBuf::from(crate::registry::DEFAULT_PATH));
        let registry_file = registry_file.or(configured.as_deref());
        crate::query::set_pseudo_resolver(Some(pseudo::resolver(
            writable_registry.clone(),
            index_file.map(Path::to_path_buf),
        )));
        let stdout = io::stdout();
        let mut out = io::BufWriter::new(stdout.lock());
        if let Command::Outdated(args) = command {
            outdated::run(&mut out, registry_file, args, quiet, verbose, || {
                self.index(index_file, quiet)
            })?;
        } else if let Command::Dependents(args) = command {
            dependents::run(&mut out, registry_file, args, None, quiet, verbose)?;
        } else if let Command::Rdependents(args) = command {
            dependents::run(
                &mut out,
                registry_file,
                args.selection,
                Some(args.full),
                quiet,
                verbose,
            )?;
        } else if let Command::Provides(args) = command {
            provides::run(&mut out, registry_file, args, quiet)?;
        } else if let Command::Location(args) = command {
            location::run(&mut out, registry_file, args, quiet)?;
        } else if let Command::Space(args) = command {
            space::run(&mut out, registry_file, args)?;
        } else if let Command::Contents(args) = command {
            contents::run(&mut out, registry_file, args, quiet)?;
        } else if let Command::Installed(args) = command {
            installed::run(&mut out, registry_file, args, quiet, verbose, || {
                self.index(index_file, quiet)
            })?;
        } else if matches!(command, Command::Version | Command::Platform) {
            match command {
                Command::Version => system::version(&mut out, quiet)?,
                Command::Platform => system::platform(&mut out, quiet)?,
                _ => unreachable!(),
            }
        } else if let Command::Download(args) = command {
            download::run(&mut out, args, quiet)?;
        } else if let Command::Fetch(_)
        | Command::Checksum(_)
        | Command::Extract(_)
        | Command::Patch(_)
        | Command::Configure(_)
        | Command::Build(_)
        | Command::Destroot(_)
        | Command::Test(_) = command
        {
            let (target, args) = match command {
                Command::Fetch(args) => {
                    crate::options::set_action_flags("fetch", &[("no-mirrors", args.no_mirrors)]);
                    ("fetch", args.phase)
                }
                Command::Checksum(args) => ("checksum", args),
                Command::Extract(args) => ("extract", args),
                Command::Patch(args) => ("patch", args),
                Command::Configure(args) => ("configure", args),
                Command::Build(args) => ("build", args),
                Command::Destroot(args) => ("destroot", args),
                Command::Test(args) => ("test", args),
                _ => unreachable!(),
            };
            let root = local::root(index_file, false, target)?.unwrap();
            phase::run(
                self.index(index_file, quiet)?,
                registry_file,
                args,
                target,
                quiet,
                &root,
            )?;
        } else if let Command::Setrequested(_)
        | Command::Unsetrequested(_)
        | Command::Setunrequested(_) = command
        {
            let path = writable_registry.clone();
            match command {
                Command::Setrequested(args) => portcmds::set_requested(&path, args, true)?,
                Command::Unsetrequested(args) | Command::Setunrequested(args) => {
                    portcmds::set_requested(&path, args, false)?
                }
                _ => unreachable!(),
            }
        } else if let Command::Cat(_)
        | Command::Edit(_)
        | Command::Dir(_)
        | Command::Work(_)
        | Command::Url(_)
        | Command::File(_)
        | Command::Logfile(_)
        | Command::Gohome(_) = command
        {
            let (action, args) = match command {
                Command::Cat(args) => ("cat", args),
                Command::Edit(args) => ("edit", args.into_ports()),
                Command::Dir(args) => ("dir", args),
                Command::Work(args) => ("work", args),
                Command::Url(args) => ("url", args),
                Command::File(args) => ("file", args),
                Command::Logfile(args) => ("logfile", args),
                Command::Gohome(args) => ("gohome", args),
                _ => unreachable!(),
            };
            let root = local::root(index_file, false, action)?.unwrap();
            portcmds::run(
                &mut out,
                self.index(index_file, quiet)?,
                action,
                args,
                &root,
            )?;
        } else if let Command::Pkg(_) | Command::Mpkg(_) | Command::Dmg(_) | Command::Mdmg(_) =
            command
        {
            let (target, args) = match command {
                Command::Pkg(args) => ("pkg", args),
                Command::Mpkg(args) => ("mpkg", args),
                Command::Dmg(args) => ("dmg", args),
                Command::Mdmg(args) => ("mdmg", args),
                _ => unreachable!(),
            };
            let root = local::root(index_file, false, target)?.unwrap();
            package::run_target(
                &mut out,
                self.index(index_file, quiet)?,
                &args.ports,
                target,
                quiet,
                &root,
            )?;
        } else if let Command::Unarchive(args) = command {
            let root = local::root(index_file, false, "unarchive")?.unwrap();
            archivefetch::unarchive(self.index(index_file, quiet)?, &args.ports, quiet, &root)?;
        } else if let Command::Archivefetch(args) = command {
            let root = local::root(index_file, false, "archivefetch")?.unwrap();
            archivefetch::run(self.index(index_file, quiet)?, &args.ports, quiet, &root)?;
        } else if let Command::Diagnose(args) = command {
            let root = local::root(index_file, false, "diagnose")?.unwrap();
            diagnose::run(args, quiet, &root)?;
        } else if let Command::Bump(args) = command {
            let root = local::root(index_file, false, "bump")?.unwrap();
            bump::run(&mut out, self.index(index_file, quiet)?, args, quiet, &root)?;
        } else if let Command::Mirror(args) = command {
            let root = local::root(index_file, false, "mirror")?.unwrap();
            mirror::run(self.index(index_file, quiet)?, args, quiet, &root)?;
        } else if let Command::Distcheck(args) = command {
            let root = local::root(index_file, false, "distcheck")?.unwrap();
            distcheck::run(
                &mut out,
                self.index(index_file, quiet)?,
                &args.ports,
                quiet,
                &root,
            )?;
        } else if let Command::Livecheck(args) = command {
            let root = local::root(index_file, false, "livecheck")?.unwrap();
            livecheck::run(
                &mut out,
                self.index(index_file, quiet)?,
                &args.ports,
                quiet,
                &root,
            )?;
        } else if let Command::Lint(args) = command {
            let root = local::root(index_file, false, "lint")?.unwrap();
            lint::run(self.index(index_file, quiet)?, args, quiet, &root)?;
        } else if let Command::Snapshot(args) = command {
            let path = writable_registry.clone();
            snapshot::run(&mut out, &path, args, quiet)?;
        } else if let Command::Restore(_) | Command::Migrate(_) = command {
            let action = if matches!(command, Command::Restore(_)) {
                "restore"
            } else {
                "migrate"
            };
            let root = local::root(index_file, false, action)?.unwrap();
            let path = writable_registry.clone();
            let index = self.index(index_file, quiet)?;
            match command {
                Command::Restore(args) => restore::run(&mut out, index, &path, args, quiet, &root)?,
                Command::Migrate(args) => {
                    restore::migrate(&mut out, index, &path, args, quiet, &root)?
                }
                _ => unreachable!(),
            }
        } else if let Command::RevUpgrade(args) = command {
            let root = local::root(index_file, false, "rev-upgrade")?.unwrap();
            let path = writable_registry.clone();
            rev_upgrade::run(
                &mut out,
                self.index(index_file, quiet)?,
                &path,
                args,
                quiet,
                &root,
            )?;
        } else if let Command::Reclaim(args) = command {
            let root = local::root(index_file, false, "reclaim")?.unwrap();
            let path = writable_registry.clone();
            reclaim::run(self.index(index_file, quiet)?, &path, args, quiet, &root)?;
        } else if let Command::Load(_) | Command::Unload(_) | Command::Reload(_) = command {
            let (action, args) = match command {
                Command::Load(args) => ("load", args),
                Command::Unload(args) => ("unload", args),
                Command::Reload(args) => ("reload", args),
                _ => unreachable!(),
            };
            let root = local::root(index_file, false, action)?.unwrap();
            launchd::run(
                self.index(index_file, quiet)?,
                &args.ports,
                action,
                quiet,
                &root,
            )?;
        } else if let Command::Distfiles(args) = command {
            let root = local::root(index_file, false, "distfiles")?.unwrap();
            distfiles::run(
                &mut out,
                self.index(index_file, quiet)?,
                &args.ports,
                quiet,
                &root,
            )?;
        } else if let Command::Log(args) = command {
            let root = local::root(index_file, false, "log")?.unwrap();
            portcmds::log(&mut out, self.index(index_file, quiet)?, args, &root)?;
        } else if let Command::Upgrade(args) = command {
            let root = local::root(index_file, false, "upgrade")?.unwrap();
            let path = writable_registry.clone();
            upgrade::run(
                &mut out,
                self.index(index_file, quiet)?,
                &path,
                args,
                quiet,
                &root,
            )?;
        } else if let Command::Clean(args) = command {
            let root = local::root(index_file, false, "clean")?.unwrap();
            clean::run(self.index(index_file, quiet)?, args, quiet, &root)?;
        } else if let Command::Notes(args) = command {
            let root = local::root(index_file, false, "notes").ok().flatten();
            portcmds::notes(
                &mut out,
                self.index(index_file, quiet)?,
                args,
                root.as_deref(),
            )?;
        } else if let Command::Activate(_) | Command::Deactivate(_) | Command::Uninstall(_) =
            command
        {
            let path = writable_registry.clone();
            let progress = crate::progress::Progress::new(quiet);
            let hooks = lifecycle::Hooks {
                root: local::root(index_file, false, "activate").ok().flatten(),
                quiet,
            };
            match command {
                Command::Activate(args) => {
                    let staging = tempfile::tempdir()?;
                    lifecycle::activate(&path, args, &hooks, &progress, staging.path())?
                }
                Command::Deactivate(args) => lifecycle::deactivate(&path, args, &hooks, &progress)?,
                Command::Uninstall(args) => lifecycle::uninstall(&path, args, &hooks, &progress)?,
                _ => unreachable!(),
            }
        } else if let Command::Variants(args) = command {
            let root = local::root(index_file, args.index, "variants")?;
            variants::run(
                &mut out,
                self.index(index_file, quiet)?,
                args,
                quiet,
                root.as_deref(),
            )?;
        } else if let Command::Deps(args) = command {
            let root = local::root(index_file, args.index, "deps")?;
            dependencies::run(
                &mut out,
                self.index(index_file, quiet)?,
                &args,
                None,
                quiet,
                verbose,
                root.as_deref(),
            )?;
        } else if let Command::Rdeps(args) = command {
            let root = local::root(index_file, args.options.index, "rdeps")?;
            dependencies::run(
                &mut out,
                self.index(index_file, quiet)?,
                &args.options,
                Some(args.full),
                quiet,
                verbose,
                root.as_deref(),
            )?;
        } else {
            // Offline index queries never initialize an async runtime or HTTP client.
            let index = self.index(index_file, quiet)?;
            match command {
                Command::Archive(args) => {
                    let root = local::root(index_file, false, "archive")?
                        .context("archive requires a local ports tree")?;
                    let specs = crate::query::specifications(index, &args.ports)?;
                    install_exec::run_specs(
                        &mut out,
                        index,
                        registry_file,
                        specs,
                        quiet,
                        &root,
                        install_exec::Mode::ARCHIVE,
                    )?
                }
                Command::Install(args) if !args.dry_run => {
                    let root = local::root(index_file, false, "install")?
                        .context("installation requires a local ports tree")?;
                    crate::options::set_action_flags(
                        "install",
                        &[
                            ("no-activate", args.no_activate),
                            ("unrequested", args.unrequested),
                            ("allow-failing", args.allow_failing),
                            ("no-replace", args.no_replace),
                            ("no-rev-upgrade", args.no_rev_upgrade),
                        ],
                    );
                    install_exec::run(&mut out, index, registry_file, &args, quiet, &root)?;
                    if !args.no_rev_upgrade && !crate::options::get().nodeps {
                        let path = registry_file
                            .map(Path::to_path_buf)
                            .unwrap_or_else(|| writable_registry.clone());
                        rev_upgrade::autorun(&mut out, index, &path, quiet, &root)?;
                    }
                }
                Command::Install(args) => {
                    let root = if let Some(path) = index_file {
                        Some(
                            path.parent()
                                .filter(|p| !p.as_os_str().is_empty())
                                .unwrap_or(Path::new(".")),
                        )
                    } else if Path::new("PortIndex").try_exists()? {
                        Some(Path::new("."))
                    } else {
                        None
                    };
                    install::run(&mut out, index, registry_file, args, quiet, root)?
                }
                Command::List { ports } => {
                    let ports = if ports.is_empty() {
                        vec!["all".into()]
                    } else {
                        ports
                    };
                    inspect::run(&mut out, index, &ports, quiet, "list")?;
                }
                Command::Echo(args) => inspect::run(&mut out, index, &args.ports, quiet, "echo")?,
                Command::Search(args) => search::run(&mut out, index, args, quiet)?,
                Command::Info(args) => info::run(&mut out, index, args, quiet, verbose)?,
                Command::Outdated(_)
                | Command::Dependents(_)
                | Command::Rdependents(_)
                | Command::Provides(_)
                | Command::Contents(_)
                | Command::Installed(_)
                | Command::Version
                | Command::Platform
                | Command::Download(_)
                | Command::Cat(_)
                | Command::Edit(_)
                | Command::Dir(_)
                | Command::Work(_)
                | Command::Url(_)
                | Command::File(_)
                | Command::Logfile(_)
                | Command::Gohome(_)
                | Command::Notes(_)
                | Command::Setrequested(_)
                | Command::Unsetrequested(_)
                | Command::Setunrequested(_)
                | Command::Activate(_)
                | Command::Deactivate(_)
                | Command::Uninstall(_)
                | Command::Fetch(_)
                | Command::Checksum(_)
                | Command::Extract(_)
                | Command::Patch(_)
                | Command::Configure(_)
                | Command::Build(_)
                | Command::Destroot(_)
                | Command::Test(_)
                | Command::Clean(_)
                | Command::Upgrade(_)
                | Command::Sync
                | Command::Usage { .. }
                | Command::Help { .. }
                | Command::Select(_)
                | Command::Log(_)
                | Command::Distfiles(_)
                | Command::Reclaim(_)
                | Command::RevUpgrade(_)
                | Command::Snapshot(_)
                | Command::Lint(_)
                | Command::Livecheck(_)
                | Command::Distcheck(_)
                | Command::Mirror(_)
                | Command::Bump(_)
                | Command::Diagnose(_)
                | Command::Archivefetch(_)
                | Command::Unarchive(_)
                | Command::Pkg(_)
                | Command::Mpkg(_)
                | Command::Dmg(_)
                | Command::Mdmg(_)
                | Command::Restore(_)
                | Command::Migrate(_)
                | Command::Load(_)
                | Command::Unload(_)
                | Command::Reload(_)
                | Command::Selfupdate { .. }
                | Command::Location(_)
                | Command::Space(_)
                | Command::Deps(_)
                | Command::Rdeps(_)
                | Command::Variants(_)
                | Command::Unsupported(_) => unreachable!(),
            }
        }
        out.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chain_reuses_catalog_and_reloads_when_source_changes() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first");
        let second = directory.path().join("second");
        for (path, name) in [(&first, "first"), (&second, "second")] {
            let body = format!("name {name} version 1");
            std::fs::write(path, format!("{name} {}\n{body}\n", body.len() + 1)).unwrap();
        }
        let mut session = Session::default();
        assert!(
            session
                .index(Some(&first), true)
                .unwrap()
                .ports()
                .contains_key("first")
        );
        std::fs::remove_file(&first).unwrap();
        assert!(
            session
                .index(Some(&first), true)
                .unwrap()
                .ports()
                .contains_key("first")
        );
        assert!(
            session
                .index(Some(&second), true)
                .unwrap()
                .ports()
                .contains_key("second")
        );
        assert!(session.index(Some(&first), true).is_err());
    }
}
