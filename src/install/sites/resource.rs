//! Isolated evaluation of the ports tree's archive_sites.tcl resource.
use anyhow::{Context, Result, anyhow, ensure};
use molt::{Interp, Value};
use std::{collections::BTreeMap, io::Read, path::Path};

const MAX_SOURCE: usize = 4 * 1024 * 1024;
const ARRAYS: [&str; 9] = [
    "sites",
    "archive_prefix",
    "archive_frameworks_dir",
    "archive_applications_dir",
    "archive_type",
    "archive_cxx_stdlib",
    "archive_delete_la_files",
    "archive_sigtype",
    "archive_pubkey",
];

/// Raw upstream arrays. URL tags and absent fields are preserved for selection.
/// Public-key paths here are configuration, never downloaded trust material.
#[derive(Debug, Default)]
pub struct Resource {
    arrays: BTreeMap<String, BTreeMap<String, String>>,
}

impl Resource {
    /// A missing resource contributes no sites, as in MacPorts base.
    pub fn read(path: &Path, platform: &str, major: u32) -> Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        let file = match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error).context("cannot open archive-site resource"),
        };
        ensure!(
            file.metadata()?.is_file(),
            "archive-site resource is not a regular file"
        );
        let mut source = String::new();
        file.take((MAX_SOURCE + 1) as u64)
            .read_to_string(&mut source)
            .with_context(|| format!("cannot read archive-site resource {}", path.display()))?;
        Self::evaluate(&source, platform, major)
            .with_context(|| format!("archive-site resource {}", path.display()))
    }

    pub fn evaluate(source: &str, platform: &str, major: u32) -> Result<Self> {
        ensure!(
            source.len() <= MAX_SOURCE,
            "archive-site resource exceeds 4 MiB"
        );
        let mut interp = Interp::new();
        interp.unset("env");
        // Only interpreter-local commands are available. In particular, a resource
        // cannot read another file, print to the terminal, or exit the process.
        for command in ["source", "exit", "puts", "parse", "pdump", "pclear", "time"] {
            interp.remove_command(command);
        }
        interp.set_recursion_limit(128);
        for (name, value) in [
            ("os.platform", platform.to_owned()),
            ("os.major", major.to_string()),
        ] {
            interp
                .set_scalar(name, Value::from(value))
                .map_err(|error| anyhow!(error.value().to_string()))?;
        }
        interp
            .eval(source)
            .map_err(|error| anyhow!("cannot evaluate archive-site resource: {}", error.value()))?;
        let arrays = ARRAYS
            .into_iter()
            .map(|name| {
                let values = interp.array_get(&format!("::portfetch::mirror_sites::{name}"));
                let entries = values
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| (pair[0].to_string(), pair[1].to_string()))
                    .collect();
                (name.to_owned(), entries)
            })
            .collect();
        Ok(Self { arrays })
    }

    pub fn value(&self, field: &str, site: &str) -> Option<&str> {
        self.arrays.get(field)?.get(site).map(String::as_str)
    }

    /// Upstream considers sites declared in archive_prefix, not arbitrary URL entries.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.arrays
            .get("archive_prefix")
            .into_iter()
            .flat_map(|array| array.keys().map(String::as_str))
    }

    pub fn urls(&self, site: &str) -> Result<Vec<String>> {
        crate::tcl_list::parse(self.value("sites", site).unwrap_or(""))
            .with_context(|| format!("archive site {site}: invalid URL list"))
    }
}
