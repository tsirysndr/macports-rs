//! `port diagnose` (macports::diagnose): check the system for common
//! problems that break MacPorts.
use crate::{progress::Progress, registry::Registry};
use anyhow::{Result, bail};
use clap::Args;
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Args)]
pub struct Diagnose {
    /// Only report problems
    #[arg(long)]
    quiet: bool,
}

struct Report<'a> {
    progress: &'a Progress,
    quiet: bool,
}
impl Report<'_> {
    fn check(&self, what: &str, ok: bool) {
        if !self.quiet {
            let result = if ok { "[SUCCESS]" } else { "[FAILED]" };
            self.progress
                .log("info", &format!("Checking for {what}... {result}"));
        }
    }
}

fn config(path: &Path, options: &mut BTreeMap<String, String>, progress: &Progress) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let known = [
        "macports_location",
        "profile_path",
        "shell_location",
        "xcode_build",
    ];
    for line in text.split('\n') {
        if line.starts_with('#') {
            continue;
        }
        let mut tokens = line.split('=');
        let key = tokens.next().unwrap_or_default();
        if known.contains(&key) || key.starts_with("xcode_version_") {
            options.insert(key.into(), tokens.next().unwrap_or_default().into());
        } else if !key.is_empty() {
            progress.error(&format!(
                "unrecognized config option in file {}: {key}",
                path.display()
            ));
        }
    }
}

fn versions(text: &str) -> Vec<String> {
    crate::tcl_list::parse(text).unwrap_or_default()
}

fn mode(path: &Path) -> Option<u32> {
    fs::metadata(path).ok().map(|m| m.permissions().mode())
}

fn check_permissions(prefix: &Path, portdbpath: &Path, sources: &[PathBuf], progress: &Progress) {
    let (mut unreadable, mut unsearchable, mut writable) = (Vec::new(), Vec::new(), Vec::new());
    let mut note = |path: &Path, mode: u32, world: bool| {
        let path = path.to_path_buf();
        if world && mode & 0o002 != 0 && !writable.contains(&path) {
            writable.push(path.clone());
        }
        if mode & 0o001 == 0 && !unsearchable.contains(&path) {
            unsearchable.push(path.clone());
        }
        if mode & 0o004 == 0 && !unreadable.contains(&path) {
            unreadable.push(path);
        }
    };
    let mut pending: Vec<PathBuf> = [
        "build",
        "distfiles",
        "incoming",
        "logs",
        "registry",
        "software",
    ]
    .iter()
    .map(|sub| portdbpath.join(sub))
    .collect();
    while let Some(path) = pending.pop() {
        let Some(bits) = mode(&path) else {
            continue;
        };
        note(&path, bits, true);
        if let Some(parent) = path.parent()
            && !pending.iter().any(|p| p == parent)
            && path != prefix
            && parent != Path::new("/")
        {
            pending.push(parent.to_path_buf());
        }
    }
    let mut pending: Vec<PathBuf> = vec![prefix.parent().unwrap_or(Path::new("/")).to_path_buf()];
    for source in sources {
        if !pending.contains(source) {
            pending.push(source.clone());
        }
    }
    while let Some(path) = pending.pop() {
        let Some(bits) = mode(&path) else {
            continue;
        };
        note(&path, bits, false);
        if let Some(parent) = path.parent()
            && !pending.iter().any(|p| p == parent)
            && parent != Path::new("/")
        {
            pending.push(parent.to_path_buf());
        }
    }
    let mut problem = false;
    for (paths, message) in [
        (
            &unreadable,
            "The following directories are not readable by all users:",
        ),
        (
            &unsearchable,
            "The following directories are not searchable by all users:",
        ),
        (
            &writable,
            "The following directories are writable by all users:",
        ),
    ] {
        if !paths.is_empty() {
            problem = true;
            progress.warning(message);
            for path in paths {
                progress.message(&format!("  {}", path.display()));
            }
        }
    }
    if problem {
        progress.warning(
            "Recommended directory permissions are 0755; use 'chmod 0755 <path>' to correct.",
        );
    }
}

fn vercmp(a: &str, b: &str) -> std::cmp::Ordering {
    crate::registry::version::compare(a, b)
}

pub(super) fn run(args: Diagnose, quiet: bool, root: &Path) -> Result<()> {
    let host = super::local::Host::new(root, None, quiet)?;
    let progress = host.progress.clone();
    let report = Report {
        progress: &progress,
        quiet: args.quiet,
    };
    let variable = |name: &str| {
        host.context
            .variables
            .get(name)
            .cloned()
            .unwrap_or_default()
    };
    let prefix = PathBuf::from(variable("prefix"));
    let portdbpath = PathBuf::from(variable("portdbpath"));
    let macos_major = variable("macos_version_major");

    let xcode_ini = root.join("_resources/macports1.0/xcode_versions.ini");
    if !xcode_ini.exists() {
        progress.error(&format!(
            "No configuration file found at {}. Please run,\n                        \"port selfupdate\"",
            xcode_ini.display()
        ));
        bail!("missing xcode_versions.ini");
    }
    let mut options = BTreeMap::new();
    config(
        Path::new("/opt/local/etc/macports/port_diagnose.ini"),
        &mut options,
        &progress,
    );
    config(&xcode_ini, &mut options, &progress);
    let location = options
        .get("macports_location")
        .cloned()
        .unwrap_or_else(|| prefix.to_string_lossy().into_owned());
    let shell = match options.get("shell_location") {
        Some(shell) => shell.clone(),
        None => {
            let user = std::env::var("SUDO_USER")
                .or_else(|_| std::env::var("USER"))
                .unwrap_or_default();
            Command::new("/usr/bin/dscl")
                .args([".", "-read", &format!("/Users/{user}"), "shell"])
                .output()
                .map(|output| {
                    String::from_utf8_lossy(&output.stdout)
                        .split_whitespace()
                        .last()
                        .unwrap_or_default()
                        .to_string()
                })
                .unwrap_or_default()
        }
    };

    // PATH
    let shell_name = Path::new(&shell)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned());
    if shell_name
        .is_some_and(|name| ["bash", "csh", "ksh", "sh", "tcsh", "zsh"].contains(&name.as_str()))
    {
        let path = std::env::var("PATH").unwrap_or_default();
        let entries: Vec<&str> = path.split(':').collect();
        if !entries.contains(&format!("{location}/bin").as_str())
            || !entries.contains(&format!("{location}/sbin").as_str())
        {
            progress.warning(&format!(
                "Your $PATH environment variable does not currently include {location}/bin or {location}/sbin"
            ));
            progress.message("Please refer to the guide on how to configure your shell:");
            progress.message("  https://guide.macports.org/#installing.shell");
        }
    }

    // MacPorts' location
    let present = prefix.join("bin/port").exists();
    if !present {
        progress.error(&format!(
            "the port command was not found in {}/bin. This can potentially cause errors. It's recommended you move it back to {}/bin.",
            prefix.display(),
            prefix.display()
        ));
    }
    report.check("MacPorts' location", present);

    // archs
    let known = ["arm64", "i386", "ppc", "ppc64", "x86_64"];
    let build_arch = versions(&variable("build_arch"));
    let universal = versions(&variable("universal_archs"));
    let macosx = variable("os.subplatform") == "macosx";
    if (macosx && build_arch.len() != 1) || build_arch.len() > 1 {
        progress.warning(&format!(
            "build_arch should contain one architecture name, but it is currently '{}'",
            build_arch.join(" ")
        ));
    } else if !build_arch.iter().all(|arch| known.contains(&arch.as_str())) {
        progress.warning(&format!(
            "build_arch should be one of '{}', but it is currently '{}'",
            known.join(" "),
            build_arch.join(" ")
        ));
    }
    if macosx && !universal.is_empty() {
        if build_arch.len() == 1 && !universal.contains(&build_arch[0]) {
            progress.warning(&format!(
                "universal_archs ({}) should contain build_arch ({})",
                universal.join(" "),
                build_arch[0]
            ));
        }
        if universal.len() < 2 && !["18", "19"].contains(&variable("os.major").as_str()) {
            progress.warning(&format!(
                "universal_archs should contain at least 2 archs, but it is currently '{}'",
                universal.join(" ")
            ));
        }
        for arch in &universal {
            if !known.contains(&arch.as_str()) {
                progress.warning(&format!(
                    "universal_archs contains '{arch}' which is not one of '{}'",
                    known.join(" ")
                ));
            }
        }
    }

    let sources = super::sync::source_trees();
    check_permissions(&prefix, &portdbpath, &sources, &progress);

    // Xcode
    let xcode = crate::source::toolchain::xcode_version();
    match options.get(&format!("xcode_version_{macos_major}")) {
        None => {
            progress.warning("No Xcode version info was found for your OS version.");
            report.check("correct Xcode version", false);
        }
        Some(list) => {
            let supported: Vec<&str> = list.split_whitespace().collect();
            if supported.contains(&xcode.as_str()) {
                report.check("correct Xcode version", true);
            } else {
                let too_old = supported.iter().any(|v| vercmp(&xcode, v).is_lt());
                let too_new = !too_old
                    && options.iter().any(|(key, list)| {
                        key.strip_prefix("xcode_version_").is_some_and(|os| {
                            vercmp(os, &macos_major).is_gt()
                                && list.split_whitespace().any(|v| vercmp(&xcode, v).is_ge())
                        })
                    });
                if too_old || too_new {
                    progress.error(&format!(
                        "The installed version of Xcode, {xcode}, is not supported by MacPorts. For your currently installed system, the following versions of Xcode are supported: {list}"
                    ));
                    report.check("correct Xcode version", false);
                } else {
                    progress.warning(&format!(
                        "The installed version of Xcode, {xcode}, is newer than the versions currently known to MacPorts. It most likely works, but has not been tested. For your currently installed system, the following versions of Xcode are known to work: {list}"
                    ));
                    report.check("correct Xcode version", true);
                }
            }
        }
    }

    for app in ["curl", "rsync", "openssl"] {
        let found = Path::new("/usr/bin").join(app).exists();
        if !found {
            progress.error(&format!(
                "{app} is needed by MacPorts to function normally, but wasn't found on this system. We'd recommend installing it for continued use of MacPorts."
            ));
        }
        report.check(&format!("for '{app}'"), found);
    }

    // free disk space, as `df -g` reports the first filesystem
    let available = Command::new("/bin/df")
        .arg("-g")
        .output()
        .ok()
        .and_then(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .nth(1)
                .and_then(|line| line.split_whitespace().nth(3).map(str::to_string))
        })
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(u64::MAX);
    if available < 5 {
        progress.warning("you have less than 5 GiB of free disk space! This can cause serious errors. We recommend trying to clear out unnecessary programs and files by running 'sudo port reclaim', or manually uninstalling/deleting programs and folders on your drive.");
    }
    report.check("free disk space", available >= 5);

    if macos_major == "10.6" {
        let x11 = Path::new("/Applications/X11.app").exists();
        if x11 {
            progress.error("it seems you have Mac OS X 10.6 installed, and are using X11 from \"X11.app\". This has been known to cause issues. To fix this, please install xorg-server, by using the command 'sudo port install xorg-server', or installing it from their website, https://www.xquartz.org/releases/.");
        }
        report.check("X11.app on Mac OS X 10.6 systems", !x11);
    }

    let matches = |directory: &str, extensions: &[&str]| {
        fs::read_dir(directory).is_ok_and(|entries| {
            entries.flatten().any(|entry| {
                Path::new(&entry.file_name())
                    .extension()
                    .is_some_and(|ext| extensions.contains(&ext.to_string_lossy().as_ref()))
            })
        })
    };
    let dylibs = matches("/usr/local/lib", &["dylib", "la", "prl"]);
    if dylibs {
        progress.warning("found dylibs in your /usr/local/lib directory. These are known to cause problems. We'd recommend you remove them.");
    }
    report.check("dylibs in /usr/local/lib", !dylibs);
    let headers = matches("/usr/local/include", &["h", "hpp", "hxx"]);
    if headers {
        progress.warning("found header files in your /usr/local/include directory. These are known to cause problems. We'd recommend you remove them.");
    }
    report.check("header files in /usr/local/include", !headers);

    // archives and files of registered ports
    if let Some(registry) = Registry::open(Some(&host.registry_path)).ok().flatten() {
        let entries = registry.entries()?;
        for entry in &entries {
            let location = registry.location(entry).unwrap_or_default();
            if !Path::new(&location).exists() {
                progress.warning(&format!(
                    "couldn't find the archive for '{} @{}_{}{}'. Please uninstall and reinstall this port.",
                    entry.name, entry.version, entry.revision, entry.variants
                ));
            }
        }
        let mut active: Vec<_> = entries.iter().filter(|entry| entry.active).collect();
        active.sort_by(|a, b| a.name.cmp(&b.name));
        for entry in active {
            for file in registry.files(entry)? {
                if fs::symlink_metadata(&file).is_err() {
                    progress.warning(&format!(
                        "couldn't find file '{file}' for port '{}'. Please deactivate and reactivate the port to fix this issue.",
                        entry.name
                    ));
                }
            }
        }
    }

    let homebrew = Path::new("/usr/local/Cellar").exists();
    if homebrew {
        progress.warning("it seems you have Homebrew installed on this system -- Because Homebrew uses /usr/local, this can potentially cause issues with MacPorts. We'd recommend you either uninstall it, or move it from /usr/local for now.");
    }
    report.check("HomeBrew", !homebrew);
    let fink = Path::new("/sw").exists();
    if fink {
        progress.warning("it seems you have Fink installed on your system -- This could potentially cause issues with MacPorts. We'd recommend you either uninstall it, or move it from /sw for now.");
    }
    report.check("Fink", !fink);

    let uninstaller = Path::new("/Developer/Library/uninstall-developer-folder");
    let stray = vercmp(&xcode, "4.3").is_ge() && uninstaller.exists();
    if stray {
        progress.warning(&format!(
            "you have leftover files from an older version of Xcode. You should delete them by using, {}",
            uninstaller.display()
        ));
    }
    report.check("stray developer directory", !stray);

    // compilation
    if !Path::new("/usr/bin/cc").is_file() {
        progress.error("No compiler found at /usr/bin/cc");
        report.check("compilation errors", false);
    } else {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("test.c");
        fs::write(&source, "int main() { return 0; }\n")?;
        let output = Command::new("/usr/bin/cc")
            .arg(&source)
            .arg("-o")
            .arg(directory.path().join("main_test"))
            .output()?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !text.trim().is_empty() || !output.status.success() {
            if text.contains("couldn't create cache file") {
                progress.warning("found errors when attempting to compile file. To fix this issue, delete your tmp folder using:\n                       rm -rf $TMPDIR");
            } else {
                progress.warning(text.trim());
            }
            report.check("compilation errors", false);
        } else {
            report.check("compilation errors", true);
        }
    }

    let dyld = std::env::vars().any(|(name, _)| name.starts_with("DYLD_"));
    if dyld {
        progress.warning("found a DYLD_* environment variable. These are known to cause issues with MacPorts. Please unset the variable for the duration MacPorts is running.");
    }
    report.check("DYLD_* environment variables", !dyld);

    // command line tools
    let clt = Path::new("/Library/Developer/CommandLineTools/").exists();
    if !clt {
        progress.warning("Xcode Command Line Tools do not appear to be installed. To install them, please enter the command:\n                                    xcode-select --install");
    } else if crate::source::toolchain::clt_version() == "none" {
        progress.warning("The Xcode Command Line Tools package appears to be installed, but its receipt appears to be missing.");
        progress.warning("The Command Line Tools may be outdated, which can cause problems.");
        progress
            .warning("Please see: <https://trac.macports.org/wiki/ProblemHotlist#reinstall-clt>");
    }
    report.check("command line tools", clt);
    Ok(())
}
