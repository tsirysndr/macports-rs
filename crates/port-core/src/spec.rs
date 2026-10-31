//! Port specifications are parsed as data, without evaluating Portfiles.
use anyhow::{Result, bail, ensure};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PortSpec {
    pub name: String,
    pub version: Option<String>,
    pub variants: BTreeMap<String, bool>,
    /// Keep insertion order, matching CLI option rendering. Later values replace earlier ones.
    pub options: Vec<(String, String)>,
}
impl PortSpec {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }
    /// A negative variant following a version needs a slash to distinguish it from a version suffix.
    pub fn parse_version(&mut self, value: &str) -> Result<()> {
        ensure!(
            self.version.is_none() && self.variants.is_empty() && self.options.is_empty(),
            "version must precede variants and options"
        );
        let value = value
            .strip_prefix('@')
            .ok_or_else(|| anyhow::anyhow!("expected @version"))?;
        let split = value.find('/').or_else(|| value.find('+'));
        let (version, rest) = match split {
            Some(i) => (
                &value[..i],
                if value.as_bytes()[i] == b'/' {
                    &value[i + 1..]
                } else {
                    &value[i..]
                },
            ),
            None => (value, ""),
        };
        ensure!(
            !version.is_empty() || !rest.is_empty(),
            "empty version specification"
        );
        if !version.is_empty() {
            self.version = Some(version.into());
        }
        if !rest.is_empty() {
            self.parse_modifier(rest)?;
        }
        Ok(())
    }
    pub fn parse_modifier(&mut self, value: &str) -> Result<()> {
        if value.starts_with('@') {
            return self.parse_version(value);
        }
        if let Some((key, value)) = value.split_once('=') {
            ensure!(valid_identifier(key), "invalid port option {key:?}");
            if let Some((_, existing)) = self.options.iter_mut().find(|(k, _)| k == key) {
                *existing = value.into();
            } else {
                self.options.push((key.into(), value.into()));
            }
            return Ok(());
        }
        let mut rest = value;
        while !rest.is_empty() {
            let enabled = match rest.as_bytes()[0] {
                b'+' => true,
                b'-' => false,
                _ => bail!("invalid variant specification {value:?}"),
            };
            rest = &rest[1..];
            let end = rest.find(['+', '-']).unwrap_or(rest.len());
            let name = &rest[..end];
            ensure!(valid_identifier(name), "invalid variant name {name:?}");
            self.variants.insert(name.into(), enabled);
            rest = &rest[end..];
        }
        ensure!(!value.is_empty(), "empty variant specification");
        Ok(())
    }
    pub fn composite_version(&self) -> String {
        let mut value = self.version.clone().unwrap_or_default();
        for enabled in [true, false] {
            for (variant, &selected) in &self.variants {
                if selected == enabled {
                    value.push(if enabled { '+' } else { '-' });
                    value.push_str(variant);
                }
            }
        }
        value
    }
    /// Upstream uses an empty full version without an explicit version, even if variants are set.
    pub fn key(&self) -> (String, String) {
        (
            self.name.to_lowercase(),
            if self.version.is_some() {
                self.composite_version()
            } else {
                String::new()
            },
        )
    }
    pub fn is_unqualified(&self) -> bool {
        self.version.is_none() && self.variants.is_empty()
    }
    pub fn has_modifiers(&self) -> bool {
        !self.is_unqualified() || !self.options.is_empty()
    }
}
pub fn is_modifier(value: &str) -> bool {
    value.starts_with(['@', '+', '-'])
        || value
            .split_once('=')
            .is_some_and(|(key, _)| valid_identifier(key))
}
fn valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions_variants_and_options() {
        let mut spec = PortSpec::named("curl");
        spec.parse_version("@8.0_1+ssl+zstd-universal").unwrap();
        spec.parse_modifier("+http2-ssl").unwrap();
        spec.parse_modifier("configure.args=--with-ssl=a=b")
            .unwrap();
        spec.parse_modifier("build.jobs=4").unwrap();
        spec.parse_modifier("build.jobs=8").unwrap();
        assert_eq!(spec.composite_version(), "8.0_1+http2+zstd-ssl-universal");
        assert_eq!(
            spec.options,
            [
                ("configure.args".into(), "--with-ssl=a=b".into()),
                ("build.jobs".into(), "8".into())
            ]
        );
    }
    #[test]
    fn version_suffix_is_not_a_disabled_variant() {
        let mut spec = PortSpec::named("pkg");
        spec.parse_version("@1.0-beta").unwrap();
        assert_eq!(spec.version.as_deref(), Some("1.0-beta"));
        assert!(spec.variants.is_empty());
        let mut spec = PortSpec::named("pkg");
        spec.parse_version("@1.0/-beta").unwrap();
        assert_eq!(spec.version.as_deref(), Some("1.0"));
        assert_eq!(spec.variants.get("beta"), Some(&false));
    }
    #[test]
    fn rejects_malformed_modifiers() {
        for modifier in ["+", "-", "--help", "+123", "+a/b", "@", "bad key=value"] {
            assert!(
                PortSpec::named("x").parse_modifier(modifier).is_err(),
                "{modifier}"
            );
        }
        let mut spec = PortSpec::named("x");
        spec.parse_modifier("+ssl").unwrap();
        assert!(spec.parse_version("@1").is_err());
    }
}
