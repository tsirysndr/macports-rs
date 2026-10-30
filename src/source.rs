//! Platform-specific discovery and first-run initialization of the official index.
use crate::{index::Index, progress::Progress};
use anyhow::{Context, Result, bail, ensure};
use futures_util::StreamExt;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::AsyncWriteExt;

#[derive(Debug, PartialEq, Eq)]
pub struct Platform {
    pub darwin_major: u32,
    /// MacPorts calls both x86 and x86_64 `i386`, and Apple Silicon `arm`.
    pub architecture: &'static str,
}
impl Platform {
    pub fn from_system(system: &str, release: &str, machine: &str) -> Result<Self> {
        ensure!(
            system == "Darwin",
            "automatic MacPorts index selection requires macOS; use --index-file on {system}"
        );
        let darwin_major = release
            .split('.')
            .next()
            .context("missing Darwin version")?
            .parse::<u32>()
            .context("invalid Darwin version")?;
        ensure!(darwin_major > 0, "invalid Darwin version");
        let architecture = match machine {
            "arm64" | "aarch64" => "arm",
            "x86_64" | "i386" | "i686" => "i386",
            "powerpc" | "ppc" | "ppc64" => "powerpc",
            _ => bail!("unsupported machine architecture {machine:?}; use --index-file"),
        };
        Ok(Self {
            darwin_major,
            architecture,
        })
    }
    pub fn directory(&self) -> String {
        format!(
            "PortIndex_darwin_{}_{}",
            self.darwin_major, self.architecture
        )
    }
    pub fn url(&self) -> String {
        format!(
            "https://rsync.macports.org/macports/release/tarballs/{}/PortIndex",
            self.directory()
        )
    }
    pub fn cache_path(&self, home: &Path) -> PathBuf {
        home.join("Library/Caches/port-rs")
            .join(self.directory())
            .join("PortIndex")
    }
}

#[cfg(target_os = "macos")]
pub fn detect_platform() -> Result<Platform> {
    use std::{ffi::CStr, mem::MaybeUninit};
    let mut uts = MaybeUninit::<libc::utsname>::uninit();
    // SAFETY: uname initializes the supplied utsname on success. Fields are NUL-terminated.
    let uts = unsafe {
        ensure!(
            libc::uname(uts.as_mut_ptr()) == 0,
            "cannot detect machine: {}",
            std::io::Error::last_os_error()
        );
        uts.assume_init()
    };
    // SAFETY: these arrays are initialized NUL-terminated strings returned by uname.
    let (system, release, machine) = unsafe {
        (
            CStr::from_ptr(uts.sysname.as_ptr()).to_str()?,
            CStr::from_ptr(uts.release.as_ptr()).to_str()?,
            CStr::from_ptr(uts.machine.as_ptr()).to_str()?,
        )
    };
    let mut arm64: libc::c_int = 0;
    let mut length = std::mem::size_of_val(&arm64);
    // Detect the physical Apple Silicon CPU even when this binary runs under Rosetta.
    // SAFETY: the output pointer and its length describe a valid c_int; no new value is supplied.
    let is_arm = unsafe {
        libc::sysctlbyname(
            c"hw.optional.arm64".as_ptr(),
            (&mut arm64 as *mut libc::c_int).cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        ) == 0
            && arm64 == 1
    };
    Platform::from_system(system, release, if is_arm { "arm64" } else { machine })
}
#[cfg(not(target_os = "macos"))]
pub fn detect_platform() -> Result<Platform> {
    bail!(
        "automatic MacPorts index selection requires macOS; use --index-file on {}",
        std::env::consts::OS
    )
}

/// Explicit paths and a project-local index remain offline and are never silently replaced.
pub fn load(explicit: Option<&Path>, quiet: bool) -> Result<Index> {
    if let Some(path) = explicit {
        return Index::read(path);
    }
    if Path::new("PortIndex").try_exists()? {
        return Index::read(Path::new("PortIndex"));
    }
    let platform = detect_platform()?;
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .context("HOME is unset; use --index-file")?;
    let destination = platform.cache_path(Path::new(&home));
    if destination.try_exists()? {
        return Index::read(&destination);
    }
    let progress = Progress::new(quiet);
    progress.phase(&format!("Initializing {}", platform.directory()));
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(initialize(&destination, &platform.url(), &progress))
}

/// Download and validate before publishing. Failed initialization leaves no partial index.
/// Existing files are never overwritten, including concurrent first-run initializations.
pub async fn initialize(destination: &Path, url: &str, progress: &Progress) -> Result<Index> {
    if destination.try_exists()? {
        return Index::read(destination);
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    tokio::fs::create_dir_all(parent).await?;
    let client = reqwest::Client::builder()
        .user_agent(concat!("port-rs/", env!("CARGO_PKG_VERSION")))
        .https_only(url.starts_with("https://"))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .timeout(Duration::from_secs(300))
        .build()?;
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("cannot download index from {url}"))?
        .error_for_status()?;
    // Metadata only; authenticity currently relies on HTTPS, not MacPorts detached signatures.
    const MAX_INDEX_BYTES: u64 = 128 * 1024 * 1024;
    ensure!(
        response
            .content_length()
            .is_none_or(|n| n <= MAX_INDEX_BYTES),
        "index exceeds 128 MiB limit"
    );
    let bar = progress.file("PortIndex");
    bar.start(response.content_length());
    let temporary = tempfile::Builder::new()
        .prefix(".PortIndex-")
        .tempfile_in(parent)?;
    let mut file = tokio::fs::File::from_std(temporary.as_file().try_clone()?);
    let mut body = response.bytes_stream();
    let mut bytes = 0u64;
    while let Some(chunk) = body.next().await {
        let chunk = chunk?;
        bytes += chunk.len() as u64;
        ensure!(bytes <= MAX_INDEX_BYTES, "index exceeds 128 MiB limit");
        file.write_all(&chunk).await?;
        bar.advance(chunk.len() as u64);
    }
    file.flush().await?;
    file.sync_all().await?;
    drop(file);
    bar.message("validating index");
    let index = Index::read(temporary.path()).context("downloaded index is invalid")?;
    ensure!(
        !index.ports.is_empty(),
        "downloaded index contains no ports"
    );
    match temporary.persist_noclobber(destination) {
        Ok(_) => (),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Index::read(destination);
        }
        Err(error) => return Err(error).context("cannot publish default index"),
    }
    bar.verified();
    progress.phase(&format!(
        "Indexed {} ports in {}",
        index.ports.len(),
        destination.display()
    ));
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selects_architecture_and_os_version() {
        for (machine, expected) in [
            ("arm64", "arm"),
            ("aarch64", "arm"),
            ("x86_64", "i386"),
            ("i386", "i386"),
            ("ppc", "powerpc"),
        ] {
            let platform = Platform::from_system("Darwin", "24.6.0", machine).unwrap();
            assert_eq!(
                platform.directory(),
                format!("PortIndex_darwin_24_{expected}")
            );
            assert!(
                platform
                    .url()
                    .ends_with(&format!("/PortIndex_darwin_24_{expected}/PortIndex"))
            );
        }
    }
    #[test]
    fn platforms_have_separate_cache_files() {
        let arm = Platform::from_system("Darwin", "24.6.0", "arm64").unwrap();
        let intel = Platform::from_system("Darwin", "24.6.0", "x86_64").unwrap();
        let newer = Platform::from_system("Darwin", "25.0.0", "arm64").unwrap();
        assert_eq!(
            arm.cache_path(Path::new("/users/test")),
            Path::new("/users/test/Library/Caches/port-rs/PortIndex_darwin_24_arm/PortIndex")
        );
        assert_ne!(
            arm.cache_path(Path::new("/tmp")),
            intel.cache_path(Path::new("/tmp"))
        );
        assert_ne!(
            arm.cache_path(Path::new("/tmp")),
            newer.cache_path(Path::new("/tmp"))
        );
    }
    #[test]
    fn unknown_platforms_are_not_silently_substituted() {
        for (os, release, arch) in [
            ("Linux", "6.0", "x86_64"),
            ("Darwin", "invalid", "arm64"),
            ("Darwin", "24.0", "riscv64"),
            ("Darwin", "0", "arm64"),
        ] {
            assert!(Platform::from_system(os, release, arch).is_err());
        }
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn detects_host_without_subprocesses() {
        let platform = detect_platform().unwrap();
        assert!(platform.darwin_major > 0);
        assert!(["arm", "i386", "powerpc"].contains(&platform.architecture));
    }
}
