use super::{Layout, Site, Sites};
use anyhow::Result;
use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
};

/// Compatible default references and their effective metadata for one ports tree.
#[derive(Debug)]
pub struct Selection {
    /// Tree references followed by configured references, including duplicates.
    pub defaults: Vec<String>,
    pub warnings: Vec<String>,
    sites: BTreeMap<String, Site>,
}

impl Selection {
    /// Select site keys using the requested archive/signature tags, as upstream
    /// verify_signature does. Literal URLs and unknown classes have no site key.
    pub fn key_paths(
        &self,
        references: &[String],
        archive_type: &str,
        signature_type: &str,
    ) -> Vec<PathBuf> {
        let mut seen = HashSet::new();
        references
            .iter()
            .filter_map(|reference| {
                let name = reference.split(':').next()?;
                let tag = reference.rsplit(':').next()?;
                if tag != archive_type {
                    return None;
                }
                let site = self.get(name)?;
                if site.signature_type != signature_type {
                    return None;
                }
                let path = site.public_key.as_ref().filter(|path| !path.is_empty())?;
                seen.insert(path.clone()).then(|| PathBuf::from(path))
            })
            .collect()
    }
    pub fn new(
        tree: Sites,
        configured: Sites,
        installation: &Layout,
        mut normalize: impl FnMut(&str) -> Result<String>,
        mut supported: impl FnMut(&str) -> bool,
    ) -> Result<Self> {
        let mut selected = Self {
            defaults: Vec::new(),
            warnings: Vec::new(),
            sites: BTreeMap::new(),
        };
        // Filter each source before merging: an incompatible or empty configured
        // entry must not mask an otherwise usable ports-tree entry.
        for source in [tree, configured] {
            selected.warnings.extend(source.warnings);
            for site in source.entries {
                if site.compatible(installation, &mut normalize, &mut supported)? {
                    selected
                        .defaults
                        .push(format!("{}::{}", site.name, site.archive_type));
                    selected.sites.insert(site.name.clone(), site);
                }
            }
        }
        Ok(selected)
    }

    /// Configured metadata overrides tree metadata for the same site name.
    pub fn get(&self, name: &str) -> Option<&Site> {
        self.sites.get(name)
    }
}
