//! Native inputs for PortSystem build defaults; no helper executables required.
use super::*;
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct BuildHost {
    pub platform: Platform,
    pub macos_version: String,
    pub active_cpus: u32,
    pub memory_bytes: Option<u64>,
}
impl BuildHost {
    pub fn variables(&self) -> Result<BTreeMap<String, String>> {
        let parts = self
            .macos_version
            .split('.')
            .map(str::parse::<u32>)
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("invalid macOS product version")?;
        ensure!(
            !parts.is_empty() && parts[0] >= 10,
            "invalid macOS product version"
        );
        let major = if parts[0] == 10 {
            format!(
                "10.{}",
                parts.get(1).context("missing macOS minor version")?
            )
        } else {
            parts[0].to_string()
        };
        let deployment = if parts[0] >= 11 {
            format!("{major}.0")
        } else {
            major.clone()
        };
        let arch = match self.platform.architecture {
            "arm" => "arm64",
            "i386" => "x86_64",
            other => bail!("unsupported build architecture {other}"),
        };
        let universal = match self.platform.darwin_major {
            20.. => "arm64 x86_64",
            19 => "x86_64",
            10..=18 => "x86_64 i386",
            _ => "i386 ppc",
        };
        let mut variables = BTreeMap::from([
            ("os.platform".into(), "darwin".into()),
            ("os.subplatform".into(), "macosx".into()),
            ("os.major".into(), self.platform.darwin_major.to_string()),
            ("os.arch".into(), self.platform.architecture.into()),
            ("prefix".into(), "/opt/local".into()),
            ("macosx_deployment_target".into(), deployment),
            ("macosx_sdk_version".into(), major),
            ("build_arch".into(), arch.into()),
            ("universal_archs".into(), universal.into()),
            ("port_rs.active_cpus".into(), self.active_cpus.to_string()),
        ]);
        if let Some(mask) = startup_creation_mask() {
            variables.insert(
                "port_rs.initial_creation_mask".into(),
                format!("{mask:04o}"),
            );
        }
        if let Some(memory) = self.memory_bytes {
            variables.insert("port_rs.memory_bytes".into(), memory.to_string());
        }
        Ok(variables)
    }
}

#[cfg(target_os = "macos")]
pub fn detect_build_host() -> Result<BuildHost> {
    use std::ffi::CStr;
    fn bytes(name: &CStr) -> Result<Vec<u8>> {
        let mut length = 0;
        // SAFETY: the first call queries size; name is NUL-terminated and all
        // output pointers refer to initialized writable storage.
        ensure!(
            unsafe {
                libc::sysctlbyname(
                    name.as_ptr(),
                    std::ptr::null_mut(),
                    &mut length,
                    std::ptr::null_mut(),
                    0,
                )
            } == 0,
            "cannot read {}: {}",
            name.to_string_lossy(),
            std::io::Error::last_os_error()
        );
        ensure!(length > 0 && length <= 1024, "invalid sysctl result size");
        let mut result = vec![0u8; length];
        // SAFETY: result has the queried capacity, and sysctl receives its size.
        ensure!(
            unsafe {
                libc::sysctlbyname(
                    name.as_ptr(),
                    result.as_mut_ptr().cast(),
                    &mut length,
                    std::ptr::null_mut(),
                    0,
                )
            } == 0,
            "cannot read {}: {}",
            name.to_string_lossy(),
            std::io::Error::last_os_error()
        );
        result.truncate(length);
        Ok(result)
    }
    let platform = detect_platform()?;
    let version = bytes(c"kern.osproductversion")?;
    let macos_version = CStr::from_bytes_with_nul(&version)?.to_str()?.to_owned();
    let active_cpus = bytes(c"hw.activecpu")
        .ok()
        .and_then(|b| b.try_into().ok())
        .map(u32::from_ne_bytes)
        .unwrap_or(2);
    let memory_key = if platform.darwin_major >= 23 {
        c"hw.memsize_usable"
    } else {
        c"hw.memsize"
    };
    let memory_bytes = bytes(memory_key)
        .ok()
        .and_then(|b| b.try_into().ok())
        .map(u64::from_ne_bytes);
    Ok(BuildHost {
        platform,
        macos_version,
        active_cpus,
        memory_bytes,
    })
}
#[cfg(not(target_os = "macos"))]
pub fn detect_build_host() -> Result<BuildHost> {
    bail!("automatic build configuration requires macOS")
}
