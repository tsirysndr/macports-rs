use crate::{
    dependencies::ReverseIndex,
    index::{Index, Port},
    spec::{PortSpec, is_modifier},
};
use anyhow::{Result, bail, ensure};
pub(crate) mod glob;
use glob::Glob;
use regex::{Regex, RegexBuilder};
use std::collections::BTreeMap;

pub enum Matcher {
    Glob(Glob),
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
            Ok(Self::Glob(Glob::new(pattern, case_sensitive)))
        }
    }
    pub fn matches(&self, value: &str) -> bool {
        match self {
            Self::Glob(g) => g.matches(value),
            Self::Regex(r) => r.is_match(value),
            Self::Exact(e) => e == value,
        }
    }
}

/// Evaluate a port expression, preserving input order and version-qualified identities.
pub fn specifications(index: &Index, tokens: &[String]) -> Result<Vec<PortSpec>> {
    if tokens.is_empty() {
        return Ok(Vec::new());
    }
    let mut parser = Expression {
        index,
        tokens,
        cursor: 0,
        depth: 0,
        reverse: None,
    };
    let ports = parser.sequence()?;
    ensure!(
        parser.peek().is_none(),
        "unexpected token {:?}",
        parser.peek().unwrap_or("")
    );
    Ok(ports)
}

pub fn lookup<'a>(index: &'a Index, name: &str) -> Result<&'a Port> {
    index
        .lookup(name)
        .ok_or_else(|| anyhow::anyhow!("port {name:?} not found"))
}

pub fn select<'a>(index: &'a Index, selectors: &[String]) -> Result<Vec<&'a Port>> {
    specifications(index, selectors)?
        .iter()
        .map(|s| lookup(index, &s.name))
        .collect()
}

/// Field selectors use case-insensitive regexes over serialized field values, as upstream does.
fn field_name(name: &str) -> Option<&str> {
    Some(match name {
        "category" => "categories",
        "variant" => "variants",
        "maintainer" => "maintainers",
        "platform" => "platforms",
        "subport" => "subports",
        "name" | "version" | "revision" | "epoch" | "categories" | "variants" | "maintainers"
        | "platforms" | "description" | "long_description" | "homepage" | "license" | "portdir"
        | "subports" | "replaced_by" | "depends_fetch" | "depends_extract" | "depends_patch"
        | "depends_build" | "depends_lib" | "depends_run" | "depends_test" => name,
        _ => return None,
    })
}

struct Expression<'a> {
    index: &'a Index,
    tokens: &'a [String],
    cursor: usize,
    depth: usize,
    reverse: Option<ReverseIndex>,
}
impl Expression<'_> {
    fn peek(&self) -> Option<&str> {
        self.tokens.get(self.cursor).map(String::as_str)
    }
    // Adjacent expressions form an implicit union, with lower precedence than explicit or.
    fn sequence(&mut self) -> Result<Vec<PortSpec>> {
        let mut result = self.disjunction()?;
        while self.peek().is_some_and(|t| t != ")") {
            result = union(result, self.disjunction()?);
        }
        Ok(result)
    }
    fn disjunction(&mut self) -> Result<Vec<PortSpec>> {
        let mut result = self.conjunction()?;
        while self.peek() == Some("or") {
            self.cursor += 1;
            result = union(result, self.conjunction()?);
        }
        Ok(result)
    }
    fn conjunction(&mut self) -> Result<Vec<PortSpec>> {
        let mut result = self.unary()?;
        while self.peek() == Some("and") {
            self.cursor += 1;
            // Upstream's direct set difference preserves explicitly qualified versions.
            if self.peek() == Some("not") {
                self.cursor += 1;
                result = difference(result, self.unary()?);
            } else {
                result = intersection(result, self.unary()?);
            }
        }
        Ok(result)
    }
    fn unary(&mut self) -> Result<Vec<PortSpec>> {
        ensure!(
            self.depth < 128,
            "port expression nesting exceeds 128 levels"
        );
        self.depth += 1;
        let result = match self.peek() {
            Some("not" | "!") => {
                self.cursor += 1;
                let excluded = self.unary()?;
                Ok(difference(self.all(), excluded))
            }
            Some("(") => {
                self.cursor += 1;
                let result = self.sequence()?;
                ensure!(
                    self.peek() == Some(")"),
                    "missing closing parenthesis in port expression"
                );
                self.cursor += 1;
                Ok(result)
            }
            Some("and" | "or" | ")" | ";") | None => bail!(
                "expected a port selector, found {:?}",
                self.peek().unwrap_or("end of expression")
            ),
            Some(_) => self.atom(),
        };
        self.depth -= 1;
        result
    }
    fn all(&self) -> Vec<PortSpec> {
        let mut ports: Vec<_> = self.index.ports().keys().map(PortSpec::named).collect();
        ports.sort_by(|a, b| crate::order::dictionary(&a.name, &b.name));
        ports
    }
    fn atom(&mut self) -> Result<Vec<PortSpec>> {
        let token = &self.tokens[self.cursor];
        self.cursor += 1;
        ensure!(
            !is_modifier(token),
            "port modifier {token:?} must follow a port selector"
        );
        let mut modifiers = PortSpec::default();
        let mut selected;
        if let Some((field, pattern)) = token.split_once(':') {
            if field == "rdepends" {
                if self.reverse.is_none() {
                    self.reverse = Some(ReverseIndex::build(self.index)?);
                }
                selected = self
                    .reverse
                    .as_ref()
                    .unwrap()
                    .recursive(pattern)
                    .into_iter()
                    .map(PortSpec::named)
                    .collect();
            } else {
                let fields =
                    if field == "depends" {
                        vec![
                            "depends_fetch",
                            "depends_extract",
                            "depends_patch",
                            "depends_build",
                            "depends_lib",
                            "depends_run",
                            "depends_test",
                        ]
                    } else {
                        vec![field_name(field).ok_or_else(|| {
                            anyhow::anyhow!("selector {field}: is not implemented")
                        })?]
                    };
                let matcher = Matcher::new(pattern, false, true, false)?;
                selected = self
                    .index
                    .ports()
                    .values()
                    .filter(|p| {
                        fields.iter().any(|f| {
                            // An absent field is distinct from a present empty value.
                            p.fields.get(*f).is_some_and(|v| matcher.matches(v))
                        })
                    })
                    .map(|p| PortSpec::named(&p.name))
                    .collect::<Vec<_>>();
            }
        } else {
            let (name, version) = token
                .split_once('@')
                .map_or((token.as_str(), None), |(n, v)| (n, Some(v)));
            ensure!(!name.is_empty(), "missing port name");
            ensure!(
                !matches!(
                    name,
                    "installed"
                        | "uninstalled"
                        | "active"
                        | "inactive"
                        | "actinact"
                        | "outdated"
                        | "requested"
                        | "unrequested"
                        | "leaves"
                        | "rleaves"
                        | "obsolete"
                ),
                "selector {name} requires an installed-port registry, which is not implemented"
            );
            ensure!(
                name != "current" && !name.contains('/'),
                "current-directory and Portfile path selectors are not implemented"
            );
            if let Some(version) = version {
                modifiers.parse_version(&format!("@{version}"))?;
            }
            selected = if name == "all" {
                self.all()
            } else if name.contains(['*', '?', '[', ']']) {
                let matcher = Matcher::new(name, false, false, false)?;
                self.index
                    .ports()
                    .keys()
                    .filter(|n| matcher.matches(n))
                    .map(PortSpec::named)
                    .collect()
            } else {
                vec![PortSpec::named(name)]
            };
        }
        // Sort each expansion, preserving the order of explicitly supplied selectors.
        if !token.starts_with("rdepends:")
            && (token.contains(':') || token.contains(['*', '?', '[', ']']))
        {
            selected.sort_by(|a, b| crate::order::dictionary(&a.name, &b.name));
        }
        while let Some(token) = self.peek().filter(|t| is_modifier(t)) {
            modifiers.parse_modifier(token)?;
            self.cursor += 1;
        }
        for spec in &mut selected {
            spec.version.clone_from(&modifiers.version);
            spec.variants.clone_from(&modifiers.variants);
            spec.options.clone_from(&modifiers.options);
        }
        Ok(selected)
    }
}

fn union(a: Vec<PortSpec>, b: Vec<PortSpec>) -> Vec<PortSpec> {
    let mut seen = std::collections::HashSet::new();
    a.into_iter()
        .chain(b)
        .filter(|p| seen.insert(p.key()))
        .collect()
}
fn group(ports: Vec<PortSpec>) -> BTreeMap<String, Vec<PortSpec>> {
    let mut grouped: BTreeMap<String, Vec<PortSpec>> = BTreeMap::new();
    for spec in union(ports, Vec::new()) {
        grouped
            .entry(spec.name.to_lowercase())
            .or_default()
            .push(spec);
    }
    grouped
}
fn intersection(a: Vec<PortSpec>, b: Vec<PortSpec>) -> Vec<PortSpec> {
    let grouped = group(b);
    let mut result = Vec::new();
    for spec in union(a, Vec::new()) {
        if let Some(matches) = grouped.get(&spec.name.to_lowercase()) {
            if spec.is_unqualified() {
                result.extend(matches.iter().cloned());
            } else if matches
                .iter()
                .any(|other| other.key() == spec.key() || other.key().1.is_empty())
            {
                result.push(spec);
            }
        }
    }
    result
}
fn difference(a: Vec<PortSpec>, b: Vec<PortSpec>) -> Vec<PortSpec> {
    let grouped = group(b);
    a.into_iter()
        .filter(|spec| {
            grouped
                .get(&spec.name.to_lowercase())
                .is_none_or(|matches| {
                    !spec.is_unqualified()
                        && !matches
                            .iter()
                            .any(|other| other.key() == spec.key() || other.key().1.is_empty())
                })
        })
        .collect()
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

#[cfg(test)]
mod expression_tests {
    use super::*;
    fn fixture() -> Index {
        let mut data = String::new();
        for (name, fields) in [
            (
                "curl",
                "version 8.0 categories {net www} description {Transfer data} variants {ssl universal} depends_lib port:zlib homepage https://example.org/?x=y",
            ),
            (
                "curlish",
                "version 1.0 categories net description {A helper} variants ssl depends_build port:curl",
            ),
            (
                "zlib",
                "version 1.3 categories archivers description {Compression library} license Zlib",
            ),
        ] {
            let body = format!("name {name} {fields}");
            data.push_str(&format!("{name} {}\n{body}\n", body.len() + 1));
        }
        Index::parse(&data).unwrap()
    }
    fn resolve(tokens: &[&str]) -> Result<Vec<PortSpec>> {
        specifications(
            &fixture(),
            &tokens.iter().map(|s| (*s).into()).collect::<Vec<_>>(),
        )
    }
    fn names(tokens: &[&str]) -> Vec<String> {
        resolve(tokens)
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect()
    }
    #[test]
    fn operator_precedence_and_implicit_union() {
        assert_eq!(names(&["curl", "or", "curlish", "and", "zlib"]), ["curl"]);
        assert_eq!(names(&["curl", "zlib", "and", "curlish"]), ["curl"]);
        assert_eq!(
            names(&["(", "curl", "or", "curlish", ")", "and", "name:ish$"]),
            ["curlish"]
        );
    }
    #[test]
    fn complement_and_grouping() {
        assert_eq!(names(&["all", "and", "not", "curl*"]), ["zlib"]);
        assert_eq!(
            names(&["!", "(", "category:net", "or", "zlib", ")"]),
            Vec::<String>::new()
        );
        assert_eq!(names(&["not", "not", "curl"]), ["curl"]);
    }
    #[test]
    fn selectors_are_regexes_with_singular_plural_aliases() {
        assert_eq!(names(&["name:^CURL$"]), ["curl"]);
        assert_eq!(names(&["categories:^net$"]), ["curlish"]);
        assert_eq!(
            names(&["category:net", "and", "variant:universal"]),
            ["curl"]
        );
        assert_eq!(names(&["description:^Compression"]), ["zlib"]);
        assert_eq!(names(&["version:^1\\."]), ["curlish", "zlib"]);
        assert_eq!(names(&["depends:(curl|zlib)$"]), ["curl", "curlish"]);
        assert_eq!(names(&["homepage:x=y"]), ["curl"]);
        assert!(names(&["epoch:^$"]).is_empty());
    }
    #[test]
    fn selectors_can_match_empty_sets() {
        assert!(names(&["name:does-not-exist"]).is_empty());
        assert!(names(&["missing*"]).is_empty());
        assert_eq!(
            names(&["not", "name:does-not-exist"]),
            ["curl", "curlish", "zlib"]
        );
    }
    #[test]
    fn union_preserves_order_and_first_duplicate() {
        assert_eq!(names(&["zlib", "curl", "zlib"]), ["zlib", "curl"]);
        let specs = resolve(&["curl", "+ssl", "curl", "-ssl"]).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].variants.get("ssl"), Some(&true));
        assert_eq!(resolve(&["curl@1", "curl@2"]).unwrap().len(), 2);
    }
    #[test]
    fn qualified_intersection_and_difference() {
        assert_eq!(
            resolve(&["curl", "and", "curl@1+ssl"]).unwrap()[0].composite_version(),
            "1+ssl"
        );
        assert_eq!(
            resolve(&["curl@1+ssl", "and", "curl"]).unwrap()[0].composite_version(),
            "1+ssl"
        );
        assert!(resolve(&["curl@1", "and", "curl@2"]).unwrap().is_empty());
        assert!(
            resolve(&["curl", "and", "not", "curl@2"])
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            resolve(&["curl@1", "and", "not", "curl@2"]).unwrap()[0].composite_version(),
            "1"
        );
    }
    #[test]
    fn modifiers_apply_to_each_expanded_port_only() {
        let specs = resolve(&[
            "category:net",
            "@8.0",
            "+ssl-universal",
            "build.jobs=8",
            "zlib",
        ])
        .unwrap();
        assert_eq!(specs.len(), 3);
        for spec in &specs[..2] {
            assert_eq!(spec.composite_version(), "8.0+ssl-universal");
            assert_eq!(spec.options[0], ("build.jobs".into(), "8".into()));
        }
        assert!(!specs[2].has_modifiers());
    }
    #[test]
    fn rejects_incomplete_and_unsupported_expressions() {
        for tokens in [
            vec!["curl", "and"],
            vec!["or", "curl"],
            vec!["not"],
            vec!["(", "all"],
            vec!["all", ")"],
            vec!["(", ")"],
            vec!["name:["],
            vec!["curl", ";", "list"],
            vec!["installed"],
            vec!["current"],
            vec!["depof:curl"],
            vec!["+ssl"],
        ] {
            assert!(resolve(&tokens).is_err(), "{tokens:?}");
        }
    }
    #[test]
    fn nesting_is_bounded() {
        let mut tokens = vec!["not"; 129];
        tokens.push("curl");
        assert!(resolve(&tokens).is_err());
    }
}
