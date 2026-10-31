//! Read-only dependency inspection from PortIndex metadata.
//! These fallback port names do not account for installed bin/lib/path providers.
mod graph;
mod render;
mod reverse;

pub use graph::Graph;
pub use render::{Direct, OutputOptions, direct, recursive};
pub use reverse::ReverseIndex;

use crate::{fields, index::Port, tcl_list};
use anyhow::{Context, Result, ensure};

#[derive(Clone, Copy, Debug, Default)]
pub struct Filter {
    pub no_build: bool,
    pub no_test: bool,
}
impl Filter {
    pub fn fields(self) -> impl Iterator<Item = &'static str> {
        fields::DEPENDENCIES
            .iter()
            .copied()
            .filter(move |field| match *field {
                "depends_lib" | "depends_run" => true,
                "depends_test" => !self.no_test,
                _ => !self.no_build,
            })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dependency {
    pub specification: String,
    pub name: String,
}
impl Dependency {
    pub fn parse(specification: &str) -> Result<Self> {
        // Indexed inspection extracts the fallback port name; it does not validate
        // the provider grammar used when actually evaluating/installing a port.
        // Real indexes include forms such as `port:bin/cmake:cmake`.
        let name = specification.rsplit(':').next().unwrap_or("");
        ensure!(
            !name.is_empty(),
            "invalid dependency specification {specification:?}"
        );
        ensure!(
            !name.chars().any(char::is_whitespace),
            "invalid dependency port name {name:?}"
        );
        Ok(Self {
            specification: specification.into(),
            name: name.into(),
        })
    }
    pub fn display(&self, verbose: bool) -> &str {
        if verbose {
            &self.specification
        } else {
            &self.name
        }
    }
}

pub fn in_field(port: &Port, field: &str) -> Result<Vec<Dependency>> {
    tcl_list::parse(port.get(field))
        .with_context(|| format!("{}: {field}", port.name))?
        .iter()
        .map(|raw| Dependency::parse(raw).with_context(|| format!("{}: {field}", port.name)))
        .collect()
}
pub fn of(port: &Port, filter: Filter) -> Result<Vec<Dependency>> {
    let mut result = Vec::new();
    for field in filter.fields() {
        result.extend(in_field(port, field)?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_supported_dependency_forms() {
        for (raw, name) in [
            ("port:zlib", "zlib"),
            ("bin:make:gmake", "gmake"),
            ("lib:libssl:openssl", "openssl"),
            ("path:lib/pkgconfig/zlib.pc:zlib", "zlib"),
            ("port:bin/cmake:cmake", "cmake"),
            ("custom:provider:leaf", "leaf"),
            ("zlib", "zlib"),
        ] {
            let dependency = Dependency::parse(raw).unwrap();
            assert_eq!(dependency.name, name);
            assert_eq!(dependency.display(true), raw);
        }
    }
    #[test]
    fn rejects_malformed_dependency_forms() {
        for raw in ["", "port:", "path:lib/foo:", "port:two words"] {
            assert!(Dependency::parse(raw).is_err(), "{raw}");
        }
    }
    #[test]
    fn filters_all_build_time_phases_together() {
        assert_eq!(
            Filter {
                no_build: true,
                no_test: false
            }
            .fields()
            .collect::<Vec<_>>(),
            ["depends_lib", "depends_run", "depends_test"]
        );
        assert_eq!(
            Filter {
                no_build: true,
                no_test: true
            }
            .fields()
            .collect::<Vec<_>>(),
            ["depends_lib", "depends_run"]
        );
    }
}
