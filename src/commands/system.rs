//! Commands that do not need a catalog, network client, or installed registry.
use anyhow::Result;
use std::io::Write;

pub(super) fn version(out: &mut impl Write, quiet: bool) -> Result<()> {
    writeln!(
        out,
        "{}{}",
        if quiet { "" } else { "Version: " },
        env!("CARGO_PKG_VERSION")
    )?;
    Ok(())
}

pub(super) fn platform(out: &mut impl Write, quiet: bool) -> Result<()> {
    let platform = crate::source::detect_platform()?;
    writeln!(
        out,
        "{}darwin {} {}",
        if quiet { "" } else { "Platform: " },
        platform.darwin_major,
        platform.architecture
    )?;
    Ok(())
}
