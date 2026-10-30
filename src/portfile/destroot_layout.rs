//! The layout-warning portion of PortSystem destroot finish.
use super::*;
use crate::progress::Progress;
use anyhow::Context;
use std::{collections::VecDeque, fs, io, path::Path};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct DestrootLayout {
    pub violations: Vec<String>,
    pub override_enabled: bool,
}
impl Session {
    /// Inspect staged layout and emit upstream's advisory warnings. This is
    /// not an activation authorization or a replacement for destroot_finish.
    /// In particular, an absent/empty stage is checked elsewhere in finish.
    pub fn check_destroot_layout(&mut self) -> Result<DestrootLayout> {
        check_supported(&mut self.interp, self.id)?;
        ensure!(
            self.interp
                .context::<State>(self.id)
                .system_executor
                .is_some(),
            "destroot layout checks require an execution-enabled session"
        );
        let override_enabled = self.scalar("destroot.violate_mtree")? == "yes";
        let subport = self.scalar("subport")?;
        let progress = self
            .interp
            .context::<State>(self.id)
            .progress
            .clone()
            .unwrap_or_else(|| Progress::new(false));
        if override_enabled {
            progress.warning(&format!(
                "{subport} installs files outside the common directory structure."
            ));
            return Ok(DestrootLayout {
                override_enabled,
                ..DestrootLayout::default()
            });
        }
        let root = self.scalar("destroot")?;
        let prefix = self.scalar("prefix")?;
        let applications = self.scalar("applications_dir")?;
        let frameworks = self.scalar("frameworks_dir")?;
        let violations = inspect(
            Path::new(&root),
            &prefix,
            &[
                &applications,
                &frameworks,
                "/Library/LaunchAgents",
                "/Library/LaunchDaemons",
                "/Library/StartupItems",
            ],
        )?;
        for path in &violations {
            progress.warning(&format!("violation by {path}"));
        }
        if !violations.is_empty() {
            progress.warning(&format!(
                "{subport} violates the layout of the ports-filesystems!"
            ));
            progress.warning("Please fix or indicate this misbehavior (if it is intended), it will be an error in future releases!");
        }
        Ok(DestrootLayout {
            violations,
            override_enabled,
        })
    }
}
fn inspect(root: &Path, prefix: &str, allowed: &[&str]) -> Result<Vec<String>> {
    const PREFIX_CHILDREN: &[&str] = &[
        "bin",
        "etc",
        "include",
        "lib",
        "libexec",
        "sbin",
        "share",
        "src",
        "var",
        "www",
        "Applications",
        "Developer",
        "Library",
    ];
    let mut pending = VecDeque::from([String::from("/")]);
    let mut violations = Vec::new();
    while let Some(directory) = pending.pop_front() {
        for name in children(&root.join(directory.trim_start_matches('/')))? {
            let path = if directory == "/" {
                format!("/{name}")
            } else {
                format!("{directory}/{name}")
            };
            if path == prefix {
                for child in children(&root.join(path.trim_start_matches('/')))? {
                    if !PREFIX_CHILDREN.contains(&child.as_str()) {
                        violations.push(format!("{path}/{child}"));
                    }
                }
            } else if prefix.starts_with(&format!("{path}/"))
                || allowed
                    .iter()
                    .any(|allowed| allowed.starts_with(&format!("{path}/")))
            {
                pending.push_back(path);
            } else if !allowed.contains(&path.as_str()) {
                violations.push(path);
            }
        }
    }
    Ok(violations)
}
fn children(directory: &Path) -> Result<Vec<String>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(Vec::new());
        }
        Err(err) => {
            return Err(err).with_context(|| {
                format!("cannot inspect staged directory {}", directory.display())
            });
        }
    };
    let mut names = entries
        .map(|entry| {
            let name = entry?.file_name();
            name.into_string().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 staged filename")
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    names.sort();
    Ok(names)
}
