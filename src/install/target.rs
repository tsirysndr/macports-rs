//! Archive coordinates from resolved build properties, matching portutil.tcl.
//! Architecture and platform declarations must come from evaluated port metadata;
//! a PortIndex alone does not necessarily contain these build properties.
use super::{Item, fetch::Request, inspect::Contents};
use crate::{source::Platform, tcl_list};
use anyhow::{Context, Result, ensure};
use ripemd::{Digest, Ripemd160};

pub enum Architecture<'a> {
    Native,
    Noarch,
    Build(&'a str),
    Universal(&'a [&'a str]),
}
#[derive(Debug, PartialEq, Eq)]
pub struct Target {
    platform: String,
    archs: Vec<String>,
}
impl Target {
    /// `platforms` is the evaluated MacPorts platforms list. Native architecture
    /// uses physical-machine detection (including Apple Silicon under Rosetta).
    pub fn macos(
        machine: &Platform,
        platforms: &str,
        architecture: Architecture<'_>,
    ) -> Result<Self> {
        ensure!(machine.darwin_major > 0, "invalid Darwin major version");
        let mut platform = format!("darwin_{}", machine.darwin_major);
        for entry in tcl_list::parse(platforms)? {
            if entry == "any" {
                platform = "any_any".into();
                break;
            }
            let parts = tcl_list::parse(&entry)?;
            if parts
                .first()
                .is_some_and(|value| matches!(value.as_str(), "darwin" | "macosx"))
                && parts.get(1).is_some_and(|value| value == "any")
            {
                platform = "darwin_any".into();
                break;
            }
        }
        let mut archs: Vec<String> = match architecture {
            Architecture::Native => vec![
                match machine.architecture {
                    "arm" => "arm64",
                    "i386" => "x86_64",
                    other => anyhow::bail!("unsupported native archive architecture {other}"),
                }
                .into(),
            ],
            Architecture::Noarch => vec!["noarch".into()],
            Architecture::Build(arch) => {
                valid_arch(arch)?;
                vec![arch.into()]
            }
            Architecture::Universal(archs) => {
                ensure!(
                    archs.len() >= 2,
                    "universal archives require at least two architectures"
                );
                for arch in archs {
                    valid_arch(arch)?;
                }
                archs.iter().map(|arch| (*arch).into()).collect()
            }
        };
        archs.sort();
        ensure!(
            archs.windows(2).all(|pair| pair[0] != pair[1]),
            "duplicate archive architecture"
        );
        Ok(Self { platform, archs })
    }
    pub fn platform(&self) -> &str {
        &self.platform
    }
    pub fn architectures(&self) -> &[String] {
        &self.archs
    }
    pub fn filename(&self, item: &Item<'_>) -> Result<String> {
        ensure!(
            item.variants_resolved,
            "default variants for {} are unresolved",
            item.port.name
        );
        for value in [&item.port.name, item.port.get("version")] {
            ensure!(
                !value.is_empty()
                    && !matches!(value, "." | "..")
                    && !value.contains(['/', '\\'])
                    && !value.chars().any(|c| c.is_control() || c.is_whitespace()),
                "invalid archive identity component"
            );
        }
        let revision = item.port.get("revision");
        if !revision.is_empty() {
            revision
                .parse::<u64>()
                .context("invalid archive revision")?;
        }
        // Item is public: validate its variant suffix rather than trust arbitrary text.
        let mut spec = crate::spec::PortSpec::named(&item.port.name);
        if !item.variants.is_empty() {
            spec.parse_modifier(&item.variants)?;
        }
        ensure!(
            spec.options.is_empty()
                && spec.version.is_none()
                && spec.variants.values().all(|v| *v)
                && spec.composite_version() == item.variants,
            "archive variants must be canonical positive variants"
        );
        let suffix = format!(".{}.{}.tbz2", self.platform, self.archs.join("-"));
        let prefix = format!("{}-{}", item.port.name, item.port.version());
        let mut filename = format!("{prefix}{}{suffix}", item.variants);
        // Upstream leaves ten bytes for signature extensions on NAME_MAX=255.
        if filename.len() > 245 && !item.variants.is_empty() {
            let hash = Ripemd160::digest(item.variants.as_bytes());
            let hash: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
            filename = format!("{prefix}+{hash}{suffix}");
        }
        ensure!(filename.len() <= 245, "archive filename is too long");
        Ok(filename)
    }
    /// Official archive coordinates. This does not assert availability, signature
    /// validity, freshness, or that the platform declaration itself is correct.
    pub fn request(&self, item: &Item<'_>, max_bytes: u64) -> Result<Request> {
        ensure!(max_bytes > 0, "archive size limit must be positive");
        let filename = self.filename(item)?;
        let mut url = reqwest::Url::parse("https://packages.macports.org/")?;
        url.path_segments_mut()
            .expect("HTTP URL has path segments")
            .push(&item.port.name)
            .push(&filename);
        Ok(Request {
            name: item.port.name.clone(),
            urls: vec![url.into()],
            max_bytes,
        })
    }
    /// The signed +CONTENTS architecture set must agree with the requested build.
    /// Platform suitability also needs resolved metadata; it is not in +CONTENTS.
    pub fn matches(&self, contents: &Contents) -> Result<()> {
        let mut actual = contents.archs.clone();
        actual.sort();
        ensure!(
            actual == self.archs,
            "archive architectures {:?} do not match requested {:?}",
            contents.archs,
            self.archs
        );
        Ok(())
    }
}
fn valid_arch(arch: &str) -> Result<()> {
    ensure!(
        !arch.is_empty()
            && arch != "noarch"
            && arch.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'),
        "invalid build architecture {arch:?}"
    );
    Ok(())
}
