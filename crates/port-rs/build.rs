//! Build darwintrace, trace mode's interposition library, for both
//! architectures so traced Rosetta and native programs both load it.
use std::{env, path::PathBuf, process::Command};

fn main() {
    let source = "darwintrace/darwintrace.c";
    println!("cargo:rerun-if-changed={source}");
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("darwintrace.dylib");
    let built = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
        && Command::new("/usr/bin/clang")
            .args([
                "-arch",
                "arm64",
                "-arch",
                "x86_64",
                "-dynamiclib",
                "-O2",
                "-o",
            ])
            .arg(&output)
            .arg(source)
            .status()
            .is_ok_and(|status| status.success());
    if !built {
        println!("cargo:warning=darwintrace was not built; trace mode cannot report hidden files");
        std::fs::write(&output, b"").unwrap();
    }
}
