//! The filesystem hierarchy portion of destroot_start.
use super::*;
use crate::build::mtree::DirectorySpec;
use anyhow::{Context, bail};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

impl Session {
    /// Populate an already prepared, private destroot using installed MacPorts
    /// directory specifications. Cleanup, mask setup, and startup items belong
    /// to the surrounding destroot_start lifecycle.
    pub fn prepare_destroot_hierarchy(&mut self) -> Result<()> {
        check_supported(&mut self.interp, self.id)?;
        ensure!(
            self.interp
                .context::<State>(self.id)
                .system_executor
                .is_some(),
            "destroot preparation requires an execution-enabled session"
        );
        ensure!(
            unsafe { libc::getuid() != 0 && libc::geteuid() != 0 },
            "privileged mtree ownership requires the installation worker"
        );
        let root = PathBuf::from(self.scalar("destroot")?);
        ensure!(
            fs::symlink_metadata(&root)?.file_type().is_dir(),
            "destroot must be a physical directory"
        );
        let share = PathBuf::from(self.scalar("portsharepath")?).join("install");
        let prefix = stage_path(&root, &self.scalar("prefix")?)?;
        let prefix_spec = DirectorySpec::parse(&fs::read_to_string(share.join("prefix.mtree"))?)?;
        let darwin = self.scalar("os.platform")? == "darwin";
        let platform = if darwin {
            Some((
                DirectorySpec::parse(&fs::read_to_string(share.join("macosx.mtree"))?)?,
                stage_path(&root, &self.scalar("applications_dir")?)?,
                stage_path(&root, &self.scalar("frameworks_dir")?)?,
            ))
        } else {
            None
        };
        let mask = self
            .interp
            .context::<State>(self.id)
            .creation_mask
            .context("creation mask is not initialized")?;
        if let Some((spec, applications, frameworks)) = platform {
            spec.apply_unprivileged(&root)?;
            crate::build::directories::create_all(&applications, Some(mask))?;
            crate::build::directories::create_all(&frameworks, Some(mask))?;
        }
        crate::build::directories::create_all(&prefix, Some(mask))?;
        prefix_spec.apply_unprivileged(&prefix)?;
        Ok(())
    }
}
fn stage_path(root: &Path, value: &str) -> Result<PathBuf> {
    let input = Path::new(value);
    ensure!(
        input.is_absolute(),
        "destroot hierarchy path must be absolute: {value}"
    );
    let mut path = root.to_owned();
    for component in input.components() {
        match component {
            Component::RootDir => (),
            Component::Normal(name) => {
                path.push(name);
                match fs::symlink_metadata(&path) {
                    Ok(meta) => ensure!(
                        meta.file_type().is_dir(),
                        "destroot hierarchy path is not a physical directory: {}",
                        path.display()
                    ),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => (),
                    Err(err) => return Err(err.into()),
                }
            }
            _ => bail!("invalid destroot hierarchy path: {value}"),
        }
    }
    Ok(path)
}
