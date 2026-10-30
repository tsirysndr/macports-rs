//! Shared local-tree selection and explicit native evaluation variables.
use anyhow::{Result, ensure};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub(super) fn root(index: Option<&Path>, indexed: bool, action: &str) -> Result<Option<PathBuf>> {
    if indexed {
        return Ok(None);
    }
    if let Some(path) = index {
        return Ok(Some(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .to_path_buf(),
        ));
    }
    ensure!(
        Path::new("PortIndex").try_exists()?,
        "no local ports tree selected; use --index-file PATH or use {action} --index"
    );
    Ok(Some(PathBuf::from(".")))
}

pub(super) fn variables(root: Option<&Path>) -> Result<BTreeMap<String, String>> {
    if root.is_none() {
        return Ok(BTreeMap::new());
    }
    let platform = crate::source::detect_platform()?;
    Ok(BTreeMap::from([
        ("os.platform".into(), "darwin".into()),
        ("os.subplatform".into(), "macosx".into()),
        ("os.major".into(), platform.darwin_major.to_string()),
        ("os.arch".into(), platform.architecture.into()),
        ("prefix".into(), "/opt/local".into()),
    ]))
}
