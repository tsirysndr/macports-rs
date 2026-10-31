//! Lazily detected Xcode and Command Line Tools facts, cached per process.
use std::{path::Path, process::Command, sync::OnceLock};

const CLT: &str = "/Library/Developer/CommandLineTools";

fn output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// `xcode-select -print-path`, falling back to the upstream default location.
pub fn developer_dir(darwin_major: u32) -> String {
    static VALUE: OnceLock<String> = OnceLock::new();
    VALUE
        .get_or_init(|| {
            output("/usr/bin/xcode-select", &["-print-path"])
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
