//! Archive defaults captured from startup configuration for Portfile evaluation.
use super::{Config, Context, Paths};
use crate::{
    install::sites::{Layout, Selection, resource::Resource},
    portfile::ArchiveSites,
    progress::Progress,
};
use anyhow::{Context as _, Result};
use std::{cell::RefCell, collections::BTreeMap, path::Path};

impl Paths {
    pub fn archive_resolver(
        &self,
        config: &Config,
        context: &Context,
        tree: &Path,
        progress: Progress,
    ) -> Result<ArchiveSites> {
        let get = |name: &str| {
            context
                .variables
                .get(name)
                .with_context(|| format!("missing archive startup value {name}"))
        };
        let platform = get("os.platform")?.clone();
        let major = get("os.major")?
            .parse::<u32>()
            .context("invalid archive platform major version")?;
        let darwin = platform == "darwin";
        let layout = Layout {
            prefix: self.normalize(get("prefix_frozen")?)?,
            frameworks_dir: self.normalize(get("frameworks_dir_frozen")?)?,
            applications_dir: self.normalize(get("applications_dir_frozen")?)?,
            cxx_stdlib: context
                .variables
                .get("cxx_stdlib")
                .cloned()
                .unwrap_or_else(|| {
                    if darwin && major >= 10 {
                        "libc++"
                    } else if darwin {
                        "libstdc++"
                    } else {
                        ""
                    }
                    .into()
                }),
            delete_la_files: context
                .variables
                .get("delete_la_files")
                .cloned()
                .unwrap_or_else(|| if darwin && major >= 13 { "yes" } else { "no" }.into()),
        };
        let paths = self.clone();
        let config = config.clone();
        let root = tree.to_path_buf();
        // Shared by every dependency session: resource evaluation and warnings
        // happen once per resource location during this install plan.
        let cache = RefCell::new(BTreeMap::<(String, String), Vec<String>>::new());
        Ok(ArchiveSites::new(move |listpath, listfile| {
            let key = (listpath.to_owned(), listfile.to_owned());
            if let Some(defaults) = cache.borrow().get(&key) {
                return Ok(defaults.clone());
            }
            let resource = if root.is_absolute() {
                root.clone()
            } else {
                paths.working_directory.join(&root)
            }
            .join("_resources")
            .join(listpath)
            .join(listfile);
            let tree = Resource::read(&resource, &platform, major)?.sites()?;
            let configured = paths.archive_sites(&config, &platform, major)?;
            let selected = Selection::new(
                tree,
                configured,
                &layout,
                |path| paths.normalize(path),
                |kind| kind == "tbz2",
            )?;
            for warning in selected.warnings {
                progress.warning(&warning);
            }
            cache.borrow_mut().insert(key, selected.defaults.clone());
            Ok(selected.defaults)
        }))
    }
}
