//! archive_sites.conf data and installation-compatibility filtering.
use anyhow::{Context, Result};
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::OnceLock,
};

#[derive(Debug)]
pub struct Site {
    pub name: String,
    /// Configured URLs have upstream's nosubdir behavior (no port-name suffix).
    pub urls: Vec<String>,
    pub archive_type: String,
    pub signature_type: String,
    pub layout: Layout,
}
#[derive(Debug)]
pub struct Layout {
    pub prefix: String,
    pub applications_dir: String,
    pub frameworks_dir: String,
    pub cxx_stdlib: String,
    pub delete_la_files: String,
}
#[derive(Default, Debug)]
pub struct Sites {
    pub entries: Vec<Site>,
    pub warnings: Vec<String>,
}
impl Sites {
    pub fn read(path: &Path, platform: &str, major: u32) -> Result<Self> {
        match std::fs::metadata(path) {
            Ok(metadata) if metadata.is_file() => (),
            Ok(_) => return Ok(Self::default()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error).context("cannot inspect archive_sites.conf"),
        }
        Self::parse(
            &std::fs::read_to_string(path)
                .with_context(|| format!("cannot read {}", path.display()))?,
            platform,
            major,
        )
    }
    pub fn parse(source: &str, platform: &str, major: u32) -> Result<Self> {
        static LINE: OnceLock<regex::Regex> = OnceLock::new();
        let pattern = LINE.get_or_init(|| regex::Regex::new(r"^(\w+)([ \t]+(.*))?$").unwrap());
        let mut warnings = Vec::new();
        let mut names = HashMap::new();
        let mut records: Vec<(String, BTreeMap<String, String>)> = Vec::new();
        let mut current = None;
        for line in source.lines() {
            let Some(fields) = pattern.captures(line) else {
                continue;
            };
            let option = &fields[1];
            let value = fields.get(3).map_or("", |value| value.as_str());
            if option == "name" {
                current = Some(*names.entry(value.to_owned()).or_insert_with(|| {
                    records.push((value.to_owned(), BTreeMap::new()));
                    records.len() - 1
                }));
            } else if matches!(
                option,
                "applications_dir"
                    | "cxx_stdlib"
                    | "delete_la_files"
                    | "frameworks_dir"
                    | "prefix"
                    | "type"
                    | "urls"
            ) {
                if let Some(index) = current {
                    if option == "urls" {
                        crate::tcl_list::parse(value.trim()).with_context(|| {
                            format!("archive site {}: invalid URL list", records[index].0)
                        })?;
                    }
                    records[index].1.insert(option.into(), value.trim().into());
                } else {
                    warnings.push(format!(
                        "archive_sites.conf: ignoring '{option}' occurring before name"
                    ));
                }
            } else {
                warnings.push(format!(
                    "archive_sites.conf: ignoring unknown key '{option}'"
                ));
            }
        }
        let old = platform == "darwin" && major <= 12;
        let mut entries = Vec::new();
        for (name, fields) in records {
            let value =
                |key, default: &str| fields.get(key).cloned().unwrap_or_else(|| default.into());
            let prefix = value("prefix", "/opt/local");
            let urls = if let Some(urls) = fields.get("urls") {
                crate::tcl_list::parse(urls)
                    .with_context(|| format!("archive site {name}: invalid URL list"))?
            } else {
                warnings.push(format!("archive_sites.conf: no urls set for {name}"));
                Vec::new()
            };
            entries.push(Site {
                name,
                urls,
                archive_type: value("type", "tbz2"),
                signature_type: "rmd160".into(),
                layout: Layout {
                    applications_dir: value("applications_dir", "/Applications/MacPorts"),
                    frameworks_dir: value(
                        "frameworks_dir",
                        &format!("{prefix}/Library/Frameworks"),
                    ),
                    prefix,
                    cxx_stdlib: value("cxx_stdlib", if old { "libstdc++" } else { "libc++" }),
                    delete_la_files: value("delete_la_files", if old { "no" } else { "yes" }),
                },
            });
        }
        Ok(Self { entries, warnings })
    }
}
impl Site {
    /// Compare normalized site paths with the already normalized installation.
    /// Archive support is supplied by the caller's available native decoders.
    pub fn compatible(
        &self,
        installation: &Layout,
        mut normalize: impl FnMut(&str) -> Result<String>,
        supported: impl FnOnce(&str) -> bool,
    ) -> Result<bool> {
        // Normalize all paths even when URLs are empty, matching upstream order.
        let prefix = normalize(&self.layout.prefix)?;
        let frameworks = normalize(&self.layout.frameworks_dir)?;
        let applications = normalize(&self.layout.applications_dir)?;
        Ok(!self.urls.is_empty()
            && prefix == installation.prefix
            && frameworks == installation.frameworks_dir
            && applications == installation.applications_dir
            && self.layout.cxx_stdlib == installation.cxx_stdlib
            && self.layout.delete_la_files == installation.delete_la_files
            && supported(&self.archive_type))
    }
}
