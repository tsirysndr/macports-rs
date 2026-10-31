//! Formatting of indexed `port info` metadata. No Portfile evaluation occurs here.
use crate::tcl_list::dictionary as pairs;
use crate::{fields, index::Port, output::wrap, spec::PortSpec, tcl_list};
use anyhow::Result;

pub struct Options<'a> {
    pub fields: &'a [String],
    pub line: bool,
    pub pretty: bool,
    pub quiet: bool,
    pub verbose: bool,
    pub width: usize,
}
const SUMMARY: &[&str] = &[
    "heading",
    "replaced_by",
    "subports",
    "variants",
    "",
    "long_description",
    "homepage",
    "",
    "depends_fetch",
    "depends_extract",
    "depends_patch",
    "depends_build",
    "depends_lib",
    "depends_run",
    "depends_test",
    "conflicts",
    "platforms",
    "license",
    "maintainers",
];

pub fn render(port: &Port, spec: &PortSpec, options: &Options<'_>) -> Result<String> {
    let pretty = options.pretty || (options.fields.is_empty() && !options.line);
    let requested: Vec<&str> = if options.fields.is_empty() {
        SUMMARY.to_vec()
    } else {
        options.fields.iter().map(String::as_str).collect()
    };
    let mut output = Vec::new();
    let mut headings = Vec::new();
    for requested in requested {
        if requested.is_empty() {
            if pretty {
                output.push(String::new());
            }
            continue;
        }
        let field = fields::canonical(requested);
        let value = value(port, spec, field, pretty, options.line, options.verbose)?;
        if pretty {
            if value.is_empty() {
                continue;
            }
            if field == "heading" {
                headings.push(value);
            } else if field == "fullname" {
                headings.push(format!("Full Name: {value}"));
            } else {
                let label = label(field, requested);
                let prefix = format!("{label}:");
                let indent = if matches!(
                    field,
                    "description"
                        | "long_description"
                        | "homepage"
                        | "variants"
                        | "maintainers"
                        | "license"
                        | "platforms"
                        | "conflicts"
                        | "replaced_by"
                        | "subports"
                        | "patchfiles"
                ) || fields::DEPENDENCIES.contains(&field)
                {
                    22.max(prefix.chars().count() + 1)
                } else {
                    prefix.chars().count() + 1
                };
                output.push(wrap(
                    &value,
                    &format!("{prefix:<indent$}"),
                    indent,
                    options.width,
                ));
            }
        } else if options.quiet || options.line {
            output.push(value);
        } else {
            output.push(format!("{requested}: {value}"));
        }
    }
    let mut rendered = String::new();
    for heading in headings {
        rendered.push_str(&heading);
        rendered.push('\n');
    }
    if !output.is_empty() {
        rendered.push_str(&output.join(if options.line { "\t" } else { "\n" }));
        rendered.push('\n');
    }
    Ok(rendered)
}
fn value(
    port: &Port,
    spec: &PortSpec,
    field: &str,
    pretty: bool,
    line: bool,
    verbose: bool,
) -> Result<String> {
    let raw = port.get(field);
    if field == "heading" {
        let version = if port.get("revision").is_empty() || port.get("revision") == "0" {
            port.get("version").to_owned()
        } else {
            port.version()
        };
        let mut heading = format!("{} @{version}", port.name);
        if port.fields.contains_key("categories") {
            heading.push_str(&format!(
                " ({})",
                tcl_list::parse(port.get("categories"))?.join(", ")
            ));
        }
        return Ok(heading);
    }
    if field == "fullname" {
        let mut active = PortSpec::named(&port.name);
        active.version = Some(port.get("version").to_owned());
        for (variant, sign) in pairs(port.get("active_variants"))? {
            if matches!(sign.as_str(), "+" | "-") {
                active.variants.insert(variant, sign == "+");
            }
        }
        return Ok(format!("{} @{}", port.name, active.composite_version()));
    }
    if matches!(field, "description" | "long_description") {
        let text = tcl_list::parse(raw)?.join(" ");
        return Ok(if pretty {
            text
        } else {
            text.replace('\n', "\\n")
        });
    }
    if pretty && field == "license" {
        return Ok(tcl_list::parse(raw)?
            .iter()
            .map(|group| {
                let licenses = tcl_list::parse(group)?;
                Ok(if licenses.len() > 1 {
                    format!("({})", licenses.join(" or "))
                } else {
                    group.clone()
                })
            })
            .collect::<Result<Vec<_>>>()?
            .join(" and "));
    }
    let mut list = if field == "maintainers" {
        maintainers(raw, pretty)?
    } else if is_list(field) {
        tcl_list::parse(raw)?
    } else {
        return Ok(raw.into());
    };
    if field.starts_with("depends_") && !verbose {
        for dependency in &mut list {
            *dependency = dependency.rsplit(':').next().unwrap_or("").to_owned();
        }
    }
    if field == "variants" && pretty {
        list.sort();
        let defaults = pairs(port.get("vinfo"))?;
        for variant in &mut list {
            let prefix = if let Some(enabled) = spec.variants.get(variant) {
                if *enabled { "+".into() } else { "-".into() }
            } else if let Some(info) = defaults.get(variant) {
                pairs(info)?
                    .get("is_default")
                    .map(|s| format!("[{s}]"))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            *variant = format!("{prefix}{variant}");
        }
    }
    Ok(list.join(if line {
        ","
    } else if pretty && field == "maintainers" {
        "\n"
    } else {
        ", "
    }))
}
fn is_list(field: &str) -> bool {
    matches!(
        field,
        "categories" | "platforms" | "variants" | "conflicts" | "subports" | "patchfiles"
    ) || fields::DEPENDENCIES.contains(&field)
}
fn maintainers(raw: &str, pretty: bool) -> Result<Vec<String>> {
    tcl_list::parse(raw)?
        .iter()
        .map(|maintainer| {
            let (mut email, mut github, mut keyword) = (None, None, None);
            for token in tcl_list::parse(maintainer)? {
                if let Some(user) = token.strip_prefix('@') {
                    github = Some(user.to_owned());
                } else if token.contains('@') {
                    email = Some(token);
                } else if let Some((domain, user)) = token.split_once(':') {
                    email = Some(format!("{user}@{domain}"));
                } else if matches!(token.as_str(), "openmaintainer" | "nomaintainer") {
                    keyword = Some(token);
                } else {
                    email = Some(format!("{token}@macports.org"));
                }
            }
            let mut parts = Vec::new();
            if let Some(email) = email {
                parts.push(if pretty {
                    format!("Email: {email}")
                } else {
                    email
                });
            }
            if let Some(github) = github {
                parts.push(if pretty {
                    format!("GitHub: {github}")
                } else {
                    github
                });
            }
            if let Some(keyword) = keyword {
                parts.push(match (pretty, keyword.as_str()) {
                    (true, "nomaintainer") => "none".into(),
                    (false, "nomaintainer") => String::new(),
                    (true, _) => "Policy: openmaintainer".into(),
                    _ => "openmaintainer".into(),
                });
            }
            Ok(parts.join(if pretty { ", " } else { " " }))
        })
        .collect()
}
fn label<'a>(field: &str, requested: &'a str) -> &'a str {
    match field {
        "description" => "Brief Description",
        "long_description" => "Description",
        "homepage" => "Homepage",
        "variants" => "Variants",
        "maintainers" => "Maintainers",
        "license" => "License",
        "platforms" => "Platforms",
        "depends_fetch" => "Fetch Dependencies",
        "depends_extract" => "Extract Dependencies",
        "depends_patch" => "Patch Dependencies",
        "depends_build" => "Build Dependencies",
        "depends_lib" => "Library Dependencies",
        "depends_run" => "Runtime Dependencies",
        "depends_test" => "Test Dependencies",
        "conflicts" => "Conflicts with",
        "replaced_by" => "Replaced by",
        "subports" => "Sub-ports",
        "patchfiles" => "Patchfiles",
        _ => requested,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maintainer_decoding() {
        assert_eq!(
            maintainers("{example.org:dev @developer} openmaintainer", false).unwrap(),
            ["dev@example.org developer", "openmaintainer"]
        );
        assert_eq!(
            maintainers("handle nomaintainer", true).unwrap(),
            ["Email: handle@macports.org", "none"]
        );
    }
    fn test_port() -> Port {
        Port {
            name: "sample".into(),
            fields: [
                ("name", "sample"),
                ("version", "1.0"),
                ("revision", "0"),
                ("license", "{MIT BSD} Apache-2"),
                ("description", "{first\nsecond}"),
                ("variants", "ssl debug"),
                ("vinfo", "ssl {is_default +}"),
                ("active_variants", "ssl + debug -"),
            ]
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect(),
        }
    }
    #[test]
    fn formats_structured_licenses_and_default_variants() {
        let fields = vec!["license".into(), "variants".into()];
        let options = Options {
            fields: &fields,
            line: false,
            pretty: true,
            quiet: false,
            verbose: false,
            width: 80,
        };
        assert_eq!(
            render(&test_port(), &PortSpec::named("sample"), &options).unwrap(),
            "License:              (MIT or BSD) and Apache-2\nVariants:             debug, [+]ssl\n"
        );
    }
    #[test]
    fn line_output_escapes_newlines_and_builds_fullname_from_active_variants() {
        let fields = vec!["fullname".into(), "description".into(), "homepage".into()];
        let options = Options {
            fields: &fields,
            line: true,
            pretty: false,
            quiet: false,
            verbose: false,
            width: 80,
        };
        assert_eq!(
            render(&test_port(), &PortSpec::named("sample"), &options).unwrap(),
            "sample @1.0+ssl-debug\tfirst\\nsecond\t\n"
        );
    }
    #[test]
    fn malformed_variant_metadata_returns_an_error() {
        let mut port = test_port();
        port.fields.insert("vinfo".into(), "ssl".into());
        let fields = vec!["variants".into()];
        let options = Options {
            fields: &fields,
            line: false,
            pretty: true,
            quiet: false,
            verbose: false,
            width: 80,
        };
        assert!(render(&port, &PortSpec::named("sample"), &options).is_err());
    }
    #[test]
    fn wraps_with_indented_continuations() {
        assert_eq!(
            wrap("one two three\nfour", "Label: ", 7, 15),
            "Label: one two\n       three\n       four"
        );
    }
}
