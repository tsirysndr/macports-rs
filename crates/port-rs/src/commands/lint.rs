//! `port lint` (portlint): check a Portfile's source text and evaluated
//! metadata against MacPorts conventions.
use crate::{index::Index, portfile::Session, progress::Progress, query};
use anyhow::{Result, bail};
use clap::Args;
use regex::Regex;
use std::{collections::BTreeMap, path::Path};

#[derive(Args)]
pub struct Lint {
    /// Also report style issues
    #[arg(long)]
    nitpick: bool,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    ports: Vec<String>,
}

const PLATFORMS: [&str; 10] = [
    "any",
    "darwin",
    "freebsd",
    "linux",
    "macosx",
    "netbsd",
    "openbsd",
    "puredarwin",
    "solaris",
    "sunos",
];
const REQUIRED: [&str; 11] = [
    "name",
    "version",
    "description",
    "long_description",
    "categories",
    "maintainers",
    "platforms",
    "homepage",
    "master_sites",
    "checksums",
    "license",
];
const OPTIONAL: [&str; 8] = [
    "epoch",
    "revision",
    "worksrcdir",
    "distname",
    "use_automake",
    "use_autoconf",
    "use_autoreconf",
    "use_configure",
];
const CHECKSUM_TYPES: [&str; 5] = ["md5", "sha1", "rmd160", "sha256", "size"];
const SECURE_CHECKSUM_TYPES: [&str; 2] = ["rmd160", "sha256"];

struct Report<'a> {
    progress: &'a Progress,
    errors: usize,
    warnings: usize,
}
impl Report<'_> {
    fn error(&mut self, message: &str) {
        self.progress.error(message);
        self.errors += 1;
    }
    fn warn(&mut self, message: &str) {
        self.progress.warning(message);
        self.warnings += 1;
    }
    fn ok(&self, message: &str) {
        self.progress.log("info", &format!("OK: {message}"));
    }
}

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static lint pattern")
}

/// 1 for a well-formed value, 0 for a malformed one, -1 for an unknown type.
fn checksum_format(kind: &str, value: &str) -> i32 {
    let word = |length: usize| {
        value.len() == length && value.chars().all(|c| c.is_alphanumeric() || c == '_')
    };
    match kind.to_ascii_lowercase().as_str() {
        "sha256" => word(64) as i32,
        "rmd160" | "sha1" => word(40) as i32,
        "md5" => word(32) as i32,
        "size" => (!value.is_empty() && value.chars().all(|c| c.is_ascii_digit())) as i32,
        _ => -1,
    }
}

fn checksum_type_issues(types: &[String], nitpick: bool) -> Vec<String> {
    let mut issues = Vec::new();
    let mut secure = false;
    for preferred in SECURE_CHECKSUM_TYPES {
        if types.iter().any(|kind| kind == preferred) {
            secure = true;
        } else {
            issues.push(format!("missing recommended checksum type: {preferred}"));
        }
    }
    if nitpick && !types.iter().any(|kind| kind == "size") {
        issues.push("missing recommended checksum type: size".into());
    }
    if !secure {
        for kind in types {
            issues.push(format!("checksum type is insecure on its own: {kind}"));
        }
    }
    issues
}

/// `portlint::lint_checksum`: errors and warnings for a checksums value.
fn lint_checksum(text: &str, nitpick: bool) -> (Vec<String>, Vec<String>) {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let (mut errors, mut warnings) = (Vec::new(), Vec::new());
    let start = usize::from(tokens.first() == Some(&"checksum"));
    let mut prefix = String::new();
    let mut filenames = false;
    let mut types: Vec<String> = Vec::new();
    let mut index = start;
    while index < tokens.len() {
        let current = tokens[index];
        if CHECKSUM_TYPES.contains(&current) {
            let value = tokens.get(index + 1).copied().unwrap_or("");
            match checksum_format(current, value) {
                1 => types.push(current.into()),
                0 => errors.push(format!(
                    "{prefix}checksum type {current}, but checksum is invalid: {value}"
                )),
                _ => {
                    errors.push(format!("{prefix}invalid checksum type: {current} {value}"));
                    break;
                }
            }
            index += 2;
        } else if index > start && !filenames {
            errors.push(format!("invalid checksum field: {current}"));
            break;
        } else {
            if index == start {
                filenames = true;
            } else if index == tokens.len() - 1
                || checksum_format(tokens[index - 2], tokens[index - 1]) != 1
            {
                errors.push(format!("invalid checksum field: {current}"));
                break;
            }
            if !types.is_empty() {
                for issue in checksum_type_issues(&types, nitpick) {
                    warnings.push(format!("{prefix}{issue}"));
                }
            }
            prefix = format!("{current} - ");
            types.clear();
            index += 1;
        }
    }
    if !types.is_empty() {
        for issue in checksum_type_issues(&types, nitpick) {
            warnings.push(format!("{prefix}{issue}"));
        }
    }
    (errors, warnings)
}

/// License checks: hyphenated versions and the BSD spellings MacPorts uses.
fn lint_license(license: &str, report: &mut Report) {
    let mut previous = String::new();
    let text = license.replace(['{', '}'], "");
    for test in text.split([' ', '\'']).filter(|word| !word.is_empty()) {
        if test.parse::<f64>().is_ok() {
            report.error(&format!(
                "Invalid license '{previous} {test}': missing hyphen between {previous} {test}"
            ));
        } else if !test.eq_ignore_ascii_case("X11") {
            for part in test.split(['-', '\'']) {
                if part.chars().next().is_some_and(char::is_alphabetic)
                    && part
                        .chars()
                        .last()
                        .is_some_and(|c| c == '+' || c.is_ascii_digit())
                {
                    report.error(&format!(
                        "invalid license '{test}': missing hyphen before version"
                    ));
                }
            }
        }
        if test.eq_ignore_ascii_case("BSD-2") || test.eq_ignore_ascii_case("BSD-3") {
            report.error(&format!("Invalid license '{test}': use BSD instead"));
        } else if test.eq_ignore_ascii_case("BSD-4") {
            report.error(&format!("Invalid license '{test}': use BSD-old instead"));
        }
        previous = test.to_string();
    }
}

struct Source {
    portsystem: Option<String>,
    portgroups: Vec<(String, String)>,
    platforms: bool,
    variants: Vec<String>,
}

/// Line-oriented checks of the Portfile text.
fn lint_text(text: &[u8], version: &str, name: &str, nitpick: bool, report: &mut Report) -> Source {
    let mut source = Source {
        portsystem: None,
        portgroups: Vec::new(),
        platforms: false,
        variants: Vec::new(),
    };
    let trailing = regex(r"\S[ \t]+$");
    let expandtab = regex(r"\sindent-tabs-mode: nil[;\s]|[:\s](?:et|expandtab)(?:[:\s]|$)");
    let comment = regex(r"^\s*#");
    let portsystem = regex(r"^\s*PortSystem\s");
    let portsystem_version = regex(r"^\s*PortSystem\s+([0-9.]+)\s*$");
    let portgroup = regex(r"^\s*PortGroup\s");
    let portgroup_version = regex(r"^\s*PortGroup\s+([A-Za-z0-9_]+)\s+([0-9.]+)\s*$");
    let platforms = regex(r"^\s*platforms\s");
    let long_description = regex(r"^\s*long_description\s");
    let variant = regex(r"^\s*variant\s+(\w+)");
    let platform = regex(r"^\s*platform\s+(?:\w+\s+(?:\w+\s+)?)?(\w+)");
    let adduser = regex(r"^\s*adduser\s");
    let empty_configure = regex(r"^\s*configure\s+\{\s*\}");
    let eval_glob = regex(r"^(.*)(\beval\s+)(.*)(\[glob\b)(.*)$");
    let checksums = regex(r"^\s*checksums\s");
    let versioned = regex(
        r"^\s*(?:PortSystem|PortGroup|version|python\.versions|(?:perl5|php|ruby)\.branch(?:es)?|license|[A-Za-z0-9_]+\.setup)\s",
    );
    let sed = regex(r"^\s*(?:reinplace\s|system.*\Wsed\W)");
    let macosx_version = regex(r"\$\{?macosx_version");

    let mut topline = 1;
    let mut require_blank = false;
    let mut require_after = "";
    let mut in_description = false;
    let mut prohibit_tabs = false;
    let mut hashline = false;
    let ends_with_newline = text.last() == Some(&b'\n');
    let lines: Vec<&[u8]> = text.split(|byte| *byte == b'\n').collect();
    let count = if ends_with_newline {
        lines.len() - 1
    } else {
        lines.len()
    };
    for (offset, bytes) in lines.iter().take(count).enumerate() {
        let lineno = offset + 1;
        if std::str::from_utf8(bytes).is_err() {
            report.error(&format!(
                "Line {lineno} seems to contain an invalid UTF-8 sequence"
            ));
        }
        let line = String::from_utf8_lossy(bytes);
        let line = line.as_ref();
        if matches!(require_after, "PortSystem" | "PortGroup") && portgroup.is_match(line) {
            require_blank = false;
        }
        if nitpick && require_blank && !line.is_empty() {
            report.warn(&format!(
                "Line {lineno} should be a newline (after {require_after})"
            ));
        }
        require_blank = false;
        if nitpick && trailing.is_match(line) {
            report.warn(&format!(
                "Line {lineno} has trailing whitespace before newline"
            ));
        }
        if lineno == topline && line.contains("-*- ") {
            report.ok(&format!("Line {lineno} has emacs/vim Mode"));
            topline += 1;
            require_blank = true;
            require_after = "modeline";
            if expandtab.is_match(line) {
                prohibit_tabs = true;
            }
        }
        if prohibit_tabs && line.contains('\t') {
            report.warn(&format!(
                "Line {lineno} contains tab but modeline says tabs should be expanded"
            ));
        }
        if line.contains("$Id") && line.ends_with('$') {
            report.warn(&format!("Line {lineno} is using obsolete RCS tag ($Id$)"));
        }
        if comment.is_match(line) {
            continue;
        }
        if portsystem.is_match(line) {
            if source.portsystem.is_some() {
                report.error(&format!("Line {lineno} repeats PortSystem declaration"));
            }
            match portsystem_version.captures(line) {
                Some(captures) => source.portsystem = Some(captures[1].to_string()),
                None => {
                    report.error(&format!("Line {lineno} has unrecognized PortSystem"));
                    source.portsystem.get_or_insert_with(String::new);
                }
            }
            require_blank = true;
            require_after = "PortSystem";
        }
        if portgroup.is_match(line) {
            match portgroup_version.captures(line) {
                Some(captures) => {
                    if source
                        .portgroups
                        .iter()
                        .any(|(group, _)| group == &captures[1])
                    {
                        report.error(&format!(
                            "Line {lineno} repeats inclusion of PortGroup {}",
                            &captures[1]
                        ));
                    } else {
                        source
                            .portgroups
                            .push((captures[1].to_string(), captures[2].to_string()));
                    }
                }
                None => report.error(&format!("Line {lineno} has unrecognized PortGroup")),
            }
            require_blank = true;
            require_after = "PortGroup";
        }
        if platforms.is_match(line) {
            source.platforms = true;
        }
        if long_description.is_match(line) {
            in_description = true;
        }
        if in_description && !line.ends_with('\\') {
            in_description = false;
        } else if in_description {
            require_blank = false;
        }
        if let Some(captures) = variant.captures(line) {
            source.variants.push(captures[1].to_string());
        }
        if let Some(captures) = platform.captures(line) {
            let arch = &captures[1];
            for (bad, replacement) in [
                ("arm64", "arm"),
                ("intel", "i386"),
                ("ppc", "powerpc"),
                ("ppc64", "powerpc"),
                ("x86_64", "i386"),
            ] {
                if arch == bad {
                    report.error(&format!(
                        "Arch '{bad}' in platform on line {lineno} should be '{replacement}'"
                    ));
                }
            }
        }
        if adduser.is_match(line) {
            report.warn(&format!(
                "Line {lineno} calling adduser directly; consider setting add_users instead"
            ));
        }
        if empty_configure.is_match(line) {
            report.warn(&format!(
                "Line {lineno} should say \"use_configure no\" instead of declaring an empty configure phase"
            ));
        }
        if let Some(captures) = eval_glob.captures(line) {
            report.warn(&format!(
                "Line {lineno} should use the expansion operator instead of the eval procedure. Change"
            ));
            report.progress.warning(line);
            report.progress.warning("to");
            report.progress.warning(&format!(
                "{}{}{{*}}{}{}",
                &captures[1], &captures[3], &captures[4], &captures[5]
            ));
        }
        if nitpick {
            if checksums.is_match(line) {
                hashline = true;
            }
            if !hashline
                && !versioned.is_match(line)
                && !version.is_empty()
                && line.contains(version)
            {
                report.warn(&format!(
                    "Line {lineno} seems to hardcode the version number, consider using ${{version}} instead"
                ));
            }
            if hashline && !line.ends_with('\\') {
                hashline = false;
            }
        }
        if !hashline && name != "MacPorts" && line.contains("/opt/local") && !sed.is_match(line) {
            report.error(&format!(
                "Line {lineno} hardcodes /opt/local, use ${{prefix}} instead"
            ));
        }
        if macosx_version.is_match(line) {
            report.warn(&format!(
                "Line {lineno} using macosx_version; switch to macos_version or macos_version_major"
            ));
        }
    }
    if !ends_with_newline && !text.is_empty() {
        report.warn(&format!(
            "Line {count} has missing newline (at end of file)"
        ));
    }
    source
}

/// Set, or provided by a default (upstream `default` sets the variable).
fn exists(session: &mut Session, name: &str) -> bool {
    session
        .invoke("info", &["exists", name])
        .is_ok_and(|value| value == "1")
        || session.scalar(name).is_ok()
}

fn value(session: &mut Session, name: &str) -> String {
    session.scalar(name).unwrap_or_default()
}

fn list(text: &str) -> Vec<String> {
    crate::tcl_list::parse(text).unwrap_or_default()
}

fn unsafe_name(name: &str) -> bool {
    name.chars()
        .any(|c| !(c.is_alphanumeric() || "_.-".contains(c)))
}

fn lint_port(
    index: &Index,
    session: &mut Session,
    portfile: &Path,
    resources: &Path,
    nitpick: bool,
    report: &mut Report,
) -> Result<()> {
    let text = std::fs::read(portfile)?;
    let name = value(session, "name");
    let version = value(session, "version");
    let source = lint_text(&text, &version, &name, nitpick, report);

    match &source.portsystem {
        None => report.error("Didn't find PortSystem specification"),
        Some(version) if version != "1.0" => {
            report.error(&format!("Unknown PortSystem: {version}"))
        }
        Some(version) => report.ok(&format!("Found PortSystem {version}")),
    }
    for (group, version) in &source.portgroups {
        if resources
            .join(format!("port1.0/group/{group}-{version}.tcl"))
            .exists()
        {
            report.ok(&format!("Found PortGroup {group}-{version}"));
        } else {
            report.error(&format!("Unknown PortGroup: {group}-{version}"));
        }
    }
    for variable in REQUIRED {
        if variable == "master_sites" {
            let kind = value(session, "fetch.type");
            if !kind.is_empty() && kind != "standard" {
                report.ok(&format!("{variable} not required for fetch.type {kind}"));
                continue;
            }
            if list(&value(session, "distfiles")).is_empty() {
                report.ok(&format!(
                    "{variable} not required when there are no distfiles"
                ));
                continue;
            }
        }
        if exists(session, variable) {
            report.ok(&format!("Found required variable: {variable}"));
        } else {
            report.error(&format!("Missing required variable: {variable}"));
        }
    }
    for variable in OPTIONAL {
        if exists(session, variable) {
            report.ok(&format!("Found optional variable: {variable}"));
        }
    }
    if unsafe_name(&name) {
        report.error(&format!(
            "Port name '{name}' contains unsafe characters. Names should only contain alphanumeric characters, underscores, dashes or dots."
        ));
    }
    if version.contains('+') {
        report.warn(&format!(
            "Version '{version}' contains '+' character, which makes it difficult to specify on the command line due to conflict with variant syntax."
        ));
    }
    if source.platforms && exists(session, "platforms") {
        let platforms = value(session, "platforms");
        for platform in list(&platforms) {
            let platname = list(&platform).into_iter().next().unwrap_or_default();
            if !PLATFORMS.contains(&platname.as_str()) {
                report.error(&format!("Unknown platform: {platname}"));
            }
        }
        let subport = value(session, "subport");
        let subports = session
            .invoke("info", &["exists", "PortInfo(subports)"])
            .is_ok_and(|exists| exists == "1")
            && !value(session, "PortInfo(subports)").is_empty();
        if platforms == "darwin" && name.eq_ignore_ascii_case(&subport) && !subports {
            report.warn("Unnecessary platforms line as darwin is the default");
        }
    }
    let mut category = None;
    if exists(session, "categories") {
        match list(&value(session, "categories")).into_iter().next() {
            Some(primary) => {
                report.ok(&format!("Found primary category: {primary}"));
                category = Some(primary);
            }
            None => report.error("Categories list is empty"),
        }
    }
    let declarations = session.variant_declarations();
    for (number, variant) in declarations.iter().enumerate() {
        if variant.name.is_empty() {
            report.error(&format!(
                "Variant number {} does not have a name",
                number + 1
            ));
            continue;
        }
        let name_ok = variant
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.".contains(c));
        if !name_ok {
            report.error(&format!(
                "Variant name {} is not valid; use [A-Za-z0-9_.]+ only",
                variant.name
            ));
        }
        let mut description = variant.description.clone();
        let mut description_ok = true;
        if !variant.described {
            if source.variants.contains(&variant.name) && variant.global_description.is_empty() {
                report.warn(&format!(
                    "Variant {} does not have a description",
                    variant.name
                ));
                description_ok = false;
            } else if description.is_empty() {
                description = "(pre-defined variant)".into();
            }
        } else if !variant.global_description.is_empty() {
            report.warn(&format!(
                "Variant {} overrides global description",
                variant.name
            ));
        }
        for conflict in &variant.conflicts {
            if !declarations.iter().any(|other| &other.name == conflict) {
                report.warn(&format!(
                    "Variant {} conflicts with non-existing variant {conflict}",
                    variant.name
                ));
            }
        }
        if name_ok {
            if description_ok {
                report.ok(&format!("Found variant {}: {description}", variant.name));
            } else {
                report.ok(&format!("Found variant: {}", variant.name));
            }
        }
    }
    let fields = [
        "depends_fetch",
        "depends_extract",
        "depends_patch",
        "depends_lib",
        "depends_build",
        "depends_run",
        "depends_test",
    ];
    for field in fields {
        for dependency in list(&value(session, field)) {
            let target = dependency.rsplit(':').next().unwrap_or_default();
            if query::lookup(index, target).is_ok() {
                report.ok(&format!("Found dependency: {target}"));
            } else {
                report.error(&format!("Unknown dependency: {target}"));
            }
        }
    }
    for field in &fields[1..] {
        let dependencies = list(&value(session, field));
        let mut warned = Vec::new();
        for dependency in &dependencies {
            if !warned.contains(dependency)
                && dependencies
                    .iter()
                    .filter(|other| *other == dependency)
                    .count()
                    > 1
            {
                report.warn(&format!(
                    "Dependency {dependency} specified multiple times in {field}"
                ));
                warned.push(dependency.clone());
            }
        }
    }
    if exists(session, "replaced_by") {
        let replaced_by = value(session, "replaced_by");
        if unsafe_name(&replaced_by) {
            report.error(&format!(
                "replaced_by should be a single port name, invalid value: {replaced_by}"
            ));
        } else if query::lookup(index, &replaced_by).is_ok() {
            report.ok(&format!("replaced_by {replaced_by}"));
        } else {
            report.error(&format!(
                "replaced_by references unknown port: {replaced_by}"
            ));
        }
    }
    if exists(session, "checksums") {
        let (errors, warnings) = lint_checksum(&value(session, "checksums"), nitpick);
        errors.iter().for_each(|message| report.error(message));
        warnings.iter().for_each(|message| report.warn(message));
    }
    for conflict in list(&value(session, "conflicts")) {
        if unsafe_name(&conflict) {
            report.error(&format!(
                "conflicts lists invalid value, should be port name: {conflict}"
            ));
        } else if query::lookup(index, &conflict).is_ok() {
            report.ok(&format!("conflicts {conflict}"));
        } else {
            report.error(&format!("conflicts references unknown port: {conflict}"));
        }
    }
    let maintainers = value(session, "maintainers");
    if regex(r"^(.+)nomaintainer(@macports\.org)?(.+)$").is_match(&maintainers) {
        report.error("Using nomaintainer together with other maintainer");
    }
    if regex(r"^openmaintainer(@macports\.org)?$").is_match(&maintainers) {
        report.error("Using openmaintainer without any other maintainer");
    }
    let address = regex(r"^(.+)@(.+)$");
    for maintainer in list(&maintainers) {
        for entry in list(&maintainer) {
            if entry == "nomaintainer@macports.org" || entry == "openmaintainer@macports.org" {
                report.warn("Using full email address for no/open maintainer");
            } else if let Some(local) = entry.strip_suffix("@macports.org") {
                report.warn(&format!(
                    "Maintainer email address for {local} includes @macports.org"
                ));
            } else if entry == "darwinports@opendarwin.org" {
                report.warn("Using legacy email address for no/open maintainer");
            } else if let Some(captures) = address.captures(&entry) {
                report.warn(&format!(
                    "Maintainer email address should be obfuscated as {}:{}",
                    &captures[2], &captures[1]
                ));
            }
        }
    }
    let license = value(session, "license");
    if license == "unknown" {
        report.warn("no license set");
    } else {
        lint_license(&license, report);
    }
    let portdir = portfile.parent().unwrap_or(Path::new(""));
    let directory = portdir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let category_directory = portdir
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    match &category {
        Some(category) if &category_directory != category => report.error(&format!(
            "Portfile parent directory {category_directory} does not match primary category {category}"
        )),
        _ => report.ok("Portfile parent directory matches primary category"),
    }
    if directory != name {
        report.error(&format!(
            "Portfile directory {directory} does not match port name {name}"
        ));
    } else {
        report.ok("Portfile directory matches port name");
    }
    if nitpick {
        for patchfile in list(&value(session, "patchfiles")) {
            let path = Path::new(&patchfile);
            let mut extension = path.extension().map(|e| e.to_string_lossy().into_owned());
            if matches!(extension.as_deref(), Some("Z" | "gz" | "bz2" | "xz")) {
                extension = Path::new(path.file_stem().unwrap_or_default())
                    .extension()
                    .map(|e| e.to_string_lossy().into_owned());
            }
            if !matches!(extension.as_deref(), Some("diff" | "patch"))
                && portdir.join("files").join(&patchfile).exists()
            {
                report.warn(&format!(
                    "Patchfile {patchfile} does not follow the source patch naming policy \"*.diff\" or \"*.patch\""
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn run(index: &Index, args: Lint, quiet: bool, root: &Path) -> Result<()> {
    let host = super::local::Host::new(root, None, quiet)?;
    let resources = root.join("_resources");
    let mut failed = BTreeMap::new();
    for mut spec in query::specifications(index, &args.ports)? {
        host.context.variants.apply(&mut spec);
        let port = query::lookup(index, &spec.name)?;
        let mut session = host.open(root, port, &spec)?;
        let subport = value(&mut session, "subport");
        host.progress
            .phase(&format!("Verifying Portfile for {subport}"));
        let portfile = root.join(port.get("portdir")).join("Portfile");
        let mut report = Report {
            progress: &host.progress,
            errors: 0,
            warnings: 0,
        };
        lint_port(
            index,
            &mut session,
            &portfile,
            &resources,
            args.nitpick,
            &mut report,
        )?;
        host.progress.phase(&format!(
            "{} errors and {} warnings found.",
            report.errors, report.warnings
        ));
        if report.errors > 0 {
            failed.insert(subport, report.errors);
        }
    }
    if !failed.is_empty() {
        bail!(
            "lint found errors in {}",
            failed.keys().cloned().collect::<Vec<_>>().join(", ")
        );
    }
    Ok(())
}
