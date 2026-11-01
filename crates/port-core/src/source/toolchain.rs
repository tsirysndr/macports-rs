//! Lazily detected Xcode and Command Line Tools facts, cached per process.
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

const CLT: &str = "/Library/Developer/CommandLineTools";

fn output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// What `xcode-select -print-path` prints, read without running it.
fn selected_developer_dir() -> Option<PathBuf> {
    let path = match std::env::var_os("DEVELOPER_DIR").filter(|dir| !dir.is_empty()) {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            if dir.extension().is_some_and(|extension| extension == "app") {
                dir.join("Contents/Developer")
            } else {
                dir
            }
        }
        None => std::fs::read_link("/var/db/xcode_select_link").ok()?,
    };
    path.is_absolute().then_some(path)
}

/// `xcode-select -print-path`, falling back to the upstream default location.
pub fn developer_dir(darwin_major: u32) -> String {
    static VALUE: OnceLock<String> = OnceLock::new();
    VALUE
        .get_or_init(|| {
            selected_developer_dir()
                .and_then(|path| path.into_os_string().into_string().ok())
                .or_else(|| output("/usr/bin/xcode-select", &["-print-path"]))
                .filter(|path| Path::new(path).is_dir())
                .unwrap_or_else(|| {
                    if darwin_major < 11 {
                        "/Developer".into()
                    } else {
                        "/Applications/Xcode.app/Contents/Developer".into()
                    }
                })
        })
        .clone()
}

/// Xcode version from `xcodebuild -version`, or `none` without Xcode.
pub fn xcode_version() -> String {
    static VALUE: OnceLock<String> = OnceLock::new();
    VALUE
        .get_or_init(|| {
            if !Path::new("/usr/bin/xcodebuild").exists() {
                return "none".into();
            }
            if let Some(version) = bundle_xcode_version() {
                return version;
            }
            output("/usr/bin/xcodebuild", &["-version"])
                .and_then(|text| {
                    text.lines()
                        .find_map(|line| line.strip_prefix("Xcode "))
                        .map(|version| version.trim().to_string())
                })
                .unwrap_or_else(|| "none".into())
        })
        .clone()
}

/// What `xcodebuild -version` reports, from the selected Xcode's version.plist.
fn bundle_xcode_version() -> Option<String> {
    let developer = selected_developer_dir()?;
    if developer == Path::new(CLT) {
        // xcodebuild refuses to run from the Command Line Tools.
        return Some("none".into());
    }
    let contents = developer
        .parent()
        .filter(|_| {
            developer
                .file_name()
                .is_some_and(|name| name == "Developer")
        })
        .filter(|contents| contents.file_name().is_some_and(|name| name == "Contents"))?;
    contents.parent().filter(|bundle| {
        bundle
            .extension()
            .is_some_and(|extension| extension == "app")
    })?;
    let plist = std::fs::read_to_string(contents.join("version.plist")).ok()?;
    let (_, rest) = plist.split_once("<key>CFBundleShortVersionString</key>")?;
    let rest = rest.trim_start().strip_prefix("<string>")?;
    let (version, _) = rest.split_once("</string>")?;
    let version = version.trim();
    (!version.is_empty()
        && version
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.'))
    .then(|| version.to_owned())
}

pub fn xcodebuild_command() -> String {
    if xcode_version() == "none" {
        "none".into()
    } else {
        "/usr/bin/xcodebuild".into()
    }
}

/// Command Line Tools package version from pkgutil, or `none`.
pub fn clt_version() -> String {
    static VALUE: OnceLock<String> = OnceLock::new();
    VALUE
        .get_or_init(|| {
            if !Path::new(CLT).join("usr/bin/make").exists() {
                return "none".into();
            }
            for name in [
                "CLTools_Executables",
                "CLTools_Base",
                "DeveloperToolsCLI",
                "DeveloperToolsCLILeo",
            ] {
                let package = format!("com.apple.pkg.{name}");
                if let Some(info) = output("/usr/sbin/pkgutil", &["--pkg-info", &package])
                    && let Some(version) =
                        info.lines().find_map(|line| line.strip_prefix("version: "))
                {
                    return version.trim().to_string();
                }
            }
            "none".into()
        })
        .clone()
}
