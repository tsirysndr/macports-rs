use super::path;
use anyhow::{Context, Result, ensure};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
pub struct Contents {
    pub name: String,
    pub version: String,
    pub revision: u64,
    pub epoch: u64,
    pub variants: BTreeSet<String>,
    pub archs: Vec<String>,
    pub dependencies: Vec<String>,
    pub files: BTreeSet<String>,
}
impl Contents {
    pub(super) fn parse(text: &str) -> Result<Self> {
        let mut fields = BTreeMap::new();
        let mut variants = BTreeSet::new();
        let mut dependencies = Vec::new();
        let mut files = BTreeSet::new();
        let mut ignored = false;
        for line in text.lines() {
            ensure!(
                !line.chars().any(char::is_control),
                "control character in +CONTENTS"
            );
            if ignored {
                ensure!(
                    line.starts_with('+') && !line.contains('/'),
                    "@ignore must name a root control file"
                );
                ignored = false;
            } else if line == "@ignore" {
                ignored = true;
            } else if let Some(directive) = line.strip_prefix('@') {
                let (key, value) = directive.split_once(' ').unwrap_or((directive, ""));
                match key {
                    "portvariant" => {
                        let name = value.strip_prefix('+').context("invalid archive variant")?;
                        ensure!(
                            !name.is_empty()
                                && name
                                    .chars()
                                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.'),
                            "invalid archive variant"
                        );
                        ensure!(
                            variants.insert(name.to_owned()),
                            "duplicate archive variant"
                        );
                    }
                    "pkgdep" => {
                        ensure!(!value.is_empty(), "empty archive dependency");
                        dependencies.push(value.to_owned());
                    }
                    "name" | "portname" | "portversion" | "portrevision" | "portepoch"
                    | "archs" => {
                        ensure!(
                            !value.is_empty() && fields.insert(key, value).is_none(),
                            "missing or duplicate archive field {key}"
                        );
                    }
                    "comment"
                    | "macports_version"
                    | "ports_commit"
                    | "source_date_epoch"
                    | "cxx_stdlib"
                    | "cxx_stdlib_overridden" => (),
                    _ => anyhow::bail!("unsupported +CONTENTS directive @{key}"),
                }
            } else if !line.is_empty() {
                let name = path(line)?;
                ensure!(
                    !name.is_empty() && !name.starts_with('+'),
                    "invalid payload path in +CONTENTS"
                );
                ensure!(files.insert(name), "duplicate +CONTENTS path");
            }
        }
        ensure!(!ignored, "unterminated @ignore directive");
        let get = |name| {
            fields
                .get(name)
                .copied()
                .with_context(|| format!("missing +CONTENTS @{name}"))
        };
        let name = get("portname")?.to_owned();
        let version = get("portversion")?.to_owned();
        let revision = get("portrevision")?
            .parse::<u64>()
            .context("invalid archive revision")?;
        let epoch = get("portepoch")?
            .parse::<u64>()
            .context("invalid archive epoch")?;
        let suffix: String = variants.iter().map(|v| format!("+{v}")).collect();
        ensure!(
            get("name")? == format!("{name}-{version}_{revision}{suffix}"),
            "archive identity fields disagree"
        );
        let archs = get("archs")?
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        Ok(Self {
            name,
            version,
            revision,
            epoch,
            variants,
            archs,
            dependencies,
            files,
        })
    }
    /// Bind authenticated metadata to the specific indexed installation item.
    pub fn matches(&self, item: &crate::install::Item<'_>) -> Result<()> {
        let epoch = if item.port.get("epoch").is_empty() {
            0
        } else {
            item.port.get("epoch").parse::<u64>()?
        };
        let variants: String = self.variants.iter().map(|v| format!("+{v}")).collect();
        ensure!(
            self.name == item.port.name
                && self.version == item.port.get("version")
                && format!("{}_{}", self.version, self.revision) == item.port.version()
                && self.epoch == epoch
                && variants == item.variants,
            "archive does not match installation plan for {}",
            item.port.name
        );
        Ok(())
    }
}
