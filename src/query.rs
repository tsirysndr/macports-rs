use crate::{
    index::{Index, Port},
    tcl_list,
};
use anyhow::{Result, bail, ensure};
use globset::{GlobBuilder, GlobMatcher};
use regex::{Regex, RegexBuilder};
use std::collections::BTreeMap;

pub enum Matcher {
    Glob(GlobMatcher),
    Regex(Regex),
    Exact(String),
}
impl Matcher {
    pub fn new(pattern: &str, exact: bool, regex: bool, case_sensitive: bool) -> Result<Self> {
        if exact && case_sensitive {
            return Ok(Self::Exact(pattern.into()));
        }
        if exact || regex {
            let pattern = if exact {
                format!("^{}$", regex::escape(pattern))
            } else {
                pattern.into()
            };
            Ok(Self::Regex(
                RegexBuilder::new(&pattern)
                    .case_insensitive(!case_sensitive)
                    .build()?,
            ))
        } else {
            Ok(Self::Glob(
                GlobBuilder::new(pattern)
                    .case_insensitive(!case_sensitive)
                    .literal_separator(false)
                    .build()?
                    .compile_matcher(),
            ))
        }
    }
    pub fn matches(&self, value: &str) -> bool {
        match self {
            Self::Glob(g) => g.is_match(value),
            Self::Regex(r) => r.is_match(value),
            Self::Exact(e) => e == value,
        }
    }
}

pub fn select<'a>(index: &'a Index, selectors: &[String]) -> Result<Vec<&'a Port>> {
    let mut selected = BTreeMap::new();
    for selector in selectors {
        ensure!(
            !matches!(
                selector.as_str(),
                "installed"
                    | "active"
                    | "inactive"
                    | "outdated"
                    | "requested"
                    | "unrequested"
                    | "leaves"
                    | "obsolete"
            ),
            "selector {selector} requires an installed-port registry, which is not implemented"
        );
        let (field, pattern) = if selector == "all" {
            ("name", "*")
        } else if let Some((field, pattern)) = selector.split_once(':') {
            ensure!(
                matches!(
                    field,
                    "name" | "category" | "maintainer" | "variant" | "license"
                ),
                "unsupported selector {field}:"
            );
            (
                match field {
                    "category" => "categories",
                    "maintainer" => "maintainers",
                    "variant" => "variants",
                    other => other,
                },
                pattern,
            )
        } else {
            ("name", selector.as_str())
        };
        let matcher = Matcher::new(pattern, false, false, true)?;
        let mut found = false;
        for port in index.ports.values() {
            let matches = if field == "name" {
                matcher.matches(&port.name)
            } else {
                tcl_list::parse(port.get(field))?
                    .iter()
                    .any(|v| matcher.matches(v))
            };
            if matches {
                selected.insert(&port.name, port);
                found = true;
            }
        }
        if !found {
            bail!("no ports matched {selector:?}");
        }
    }
    Ok(selected.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modes() {
        assert!(
            Matcher::new("py*", false, false, false)
                .unwrap()
                .matches("Python")
        );
        assert!(
            !Matcher::new("py*", false, false, true)
                .unwrap()
                .matches("Python")
        );
        assert!(
            Matcher::new("python", true, false, false)
                .unwrap()
                .matches("Python")
        );
        assert!(
            !Matcher::new("python", true, false, false)
                .unwrap()
                .matches("python3")
        );
        assert!(
            Matcher::new("^py[0-9]+$", false, true, true)
                .unwrap()
                .matches("py313")
        );
        assert!(Matcher::new("[", false, true, true).is_err());
    }
}
