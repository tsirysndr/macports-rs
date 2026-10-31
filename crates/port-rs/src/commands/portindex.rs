//! `portindex`: evaluate every Portfile of a ports tree and write its
//! `PortIndex` and `PortIndex.quick`, as upstream's portindex tool does.
//! Unchanged Portfiles reuse their previous entries unless a full reindex is
//! asked for.
use crate::{index::Port, progress::Progress, spec::PortSpec};
use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::SystemTime,
};

const KEEP: &[&str] = &[
    "categories",
    "depends_fetch",
    "depends_extract",
    "depends_patch",
    "depends_build",
    "depends_lib",
    "depends_run",
    "depends_test",
    "description",
    "epoch",
    "homepage",
    "long_description",
    "maintainers",
    "name",
    "platforms",
    "revision",
    "variants",
    "version",
    "portdir",
    "replaced_by",
    "license",
    "installs_libs",
    "conflicts",
    "known_fail",
];

#[derive(Default)]
pub struct Options {
    /// Rebuild every entry, ignoring the previous index.
    pub full: bool,
    /// Include variant descriptions and notes (`-x`).
    pub extended: bool,
    /// Directory for PortIndex; the tree itself by default.
    pub output: Option<PathBuf>,
    /// Simulated platform, `<platform>_<major>_<arch>` (`-p`).
    pub platform: Option<String>,
    pub quiet: bool,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct Stats {
    pub total: usize,
    pub failed: usize,
    pub skipped: usize,
}

/// One port's entries: the port itself, then its subports.
type Entries = Vec<(String, String)>;

/// `<category>/<port>` directories holding a Portfile, in upstream's order.
fn port_directories(root: &Path) -> Result<Vec<String>> {
    let sorted = |path: &Path| -> Result<Vec<String>> {
        let mut names: Vec<String> = fs::read_dir(path)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        Ok(names)
    };
    let mut directories = Vec::new();
    for category in sorted(root)? {
        let path = root.join(&category);
        if category.starts_with(['_', '.']) || !path.is_dir() {
            continue;
        }
        for port in sorted(&path)? {
            if path.join(&port).join("Portfile").is_file() {
                directories.push(format!("{category}/{port}"));
            }
        }
    }
    Ok(directories)
}

/// The previous index's records by lowercase name, and its modification time.
/// Records by lowercase name: the name as written and its entry line.
type Records = HashMap<String, (String, String)>;

fn previous(path: &Path) -> Result<(Records, SystemTime)> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok((HashMap::new(), SystemTime::UNIX_EPOCH));
    };
    let mut records = HashMap::new();
    let mut rest = text.as_str();
    while let Some((header, after)) = rest.split_once('\n') {
        let mut words = header.split_whitespace();
        let (Some(name), Some(length), None) = (words.next(), words.next(), words.next()) else {
            break;
        };
        let length: usize = length.parse().context("corrupt PortIndex")?;
        let Some(end) = crate::index::record_end(after, length) else {
            break;
        };
        records.insert(
            name.to_ascii_lowercase(),
            (
                name.to_owned(),
                after[..end].trim_end_matches('\n').to_owned(),
            ),
        );
        rest = &after[end..];
    }
    Ok((records, fs::metadata(path)?.modified()?))
}

fn record(fields: &BTreeMap<String, String>, keep: &[&str], subports: bool) -> String {
    let mut pairs = Vec::new();
    for (key, value) in fields {
        let kept = keep.contains(&key.as_str()) || (subports && key == "subports");
        if kept && !value.is_empty() {
            pairs.push(molt::Value::from(key.as_str()));
            pairs.push(molt::Value::from(value.as_str()));
        }
    }
    molt::Value::from(pairs).to_string()
}

fn evaluate(
    root: &Path,
    portdir: &str,
    name: &str,
    variables: &BTreeMap<String, String>,
    environment: &BTreeMap<String, String>,
    keep: &[&str],
    subport: bool,
) -> Result<(String, String, Vec<String>)> {
    let port = Port {
        name: name.to_owned(),
        fields: BTreeMap::from([("portdir".into(), portdir.to_owned())]),
    };
    let mut evaluated = crate::portfile::open_from_tree_with_environment(
        root,
        &port,
        variables,
        &PortSpec::named(name),
        environment,
    )?
    .into_metadata();
    let fields = &mut evaluated.port.fields;
    fields.insert("portdir".into(), portdir.to_owned());
    let subports = fields
        .get("subports")
        .map(|value| crate::tcl_list::parse(value))
        .transpose()?
        .unwrap_or_default();
    let name = fields
        .get("name")
        .cloned()
        .unwrap_or_else(|| evaluated.port.name.clone());
    Ok((name, record(fields, keep, !subport), subports))
}

/// The name the Portfile declares, for directories named differently.
fn declared_name(portfile: &Path) -> Option<String> {
    let text = fs::read_to_string(portfile).ok()?;
    text.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next() == Some("name"))
            .then(|| words.next())
            .flatten()
            .filter(|name| !name.contains(['$', '[']))
            .map(String::from)
    })
}

fn index_port(
    root: &Path,
    portdir: &str,
    variables: &BTreeMap<String, String>,
    environment: &BTreeMap<String, String>,
    keep: &[&str],
) -> Result<Entries> {
    let directory_name = portdir.rsplit('/').next().unwrap_or(portdir);
    let (name, line, subports) = match evaluate(
        root,
        portdir,
        directory_name,
        variables,
        environment,
        keep,
        false,
    ) {
        Ok(result) => result,
        Err(err) => match declared_name(&root.join(portdir).join("Portfile")) {
            Some(name) if name != directory_name => {
                evaluate(root, portdir, &name, variables, environment, keep, false)?
            }
            _ => return Err(err),
        },
    };
    // Subports are only declared when evaluated under the port's own name.
    let (name, line, subports) = if name != directory_name {
        evaluate(root, portdir, &name, variables, environment, keep, false)?
    } else {
        (name, line, subports)
    };
    let mut entries = vec![(name, line)];
    for subport in subports {
        let (name, line, _) = evaluate(root, portdir, &subport, variables, environment, keep, true)
            .with_context(|| format!("subport '{subport}'"))?;
        entries.push((name, line));
    }
    Ok(entries)
}

/// Write `PortIndex.quick` (lowercase name and byte offset per entry) for an
/// existing PortIndex, as upstream's `mports_generate_quickindex`.
pub fn write_quick(index: &Path) -> Result<()> {
    let text = fs::read_to_string(index)?;
    let mut quick = String::new();
    let mut offset = 0;
    while offset < text.len() {
        let Some(end) = text[offset..].find('\n') else {
            break;
        };
        let mut words = text[offset..offset + end].split_whitespace();
        let (Some(name), Some(length), None) = (words.next(), words.next(), words.next()) else {
            break;
        };
        quick.push_str(&format!("{} {offset}\n", name.to_ascii_lowercase()));
        let body = offset + end + 1;
        let length = length.parse::<usize>().context("corrupt PortIndex")?;
        offset =
            body + crate::index::record_end(&text[body..], length).context("corrupt PortIndex")?;
    }
    let mut path = index.as_os_str().to_owned();
    path.push(".quick");
    fs::write(path, quick)?;
    Ok(())
}

pub fn run(root: &Path, options: &Options) -> Result<Stats> {
    let root = root
        .canonicalize()
        .with_context(|| format!("cannot open ports tree {}", root.display()))?;
    let progress = Progress::new(options.quiet);
    let output = options.output.clone().unwrap_or_else(|| root.clone());
    fs::create_dir_all(&output)?;
    let index_path = output.join("PortIndex");
    let (old, old_time) = if options.full {
        (HashMap::new(), SystemTime::UNIX_EPOCH)
    } else {
        previous(&index_path)?
    };
    let host = super::local::Host::new(&root, None, true)?;
    let mut variables = host.context.variables.clone();
    let environment = &host.context.environment;
    // Keep ${prefix} literal so the index works with any prefix.
    variables.insert("prefix".into(), "${prefix}".into());
    if let Some(platform) = &options.platform {
        let parts: Vec<&str> = platform.split('_').collect();
        let [platform, major, arch] = parts[..] else {
            bail!("Platform specifier should be of the form plat_ver_arch");
        };
        let platform = if platform == "macosx" {
            variables.insert("os.subplatform".into(), "macosx".into());
            let stdlib = if major.parse::<u32>().unwrap_or(0) < 10 {
                "libstdc++"
            } else {
                "libc++"
            };
            variables.insert("cxx_stdlib".into(), stdlib.into());
            "darwin"
        } else {
            platform
        };
        variables.insert("os.platform".into(), platform.into());
        variables.insert("os.major".into(), major.into());
        variables.insert("os.version".into(), format!("{major}.0.0"));
        variables.insert("os.arch".into(), arch.into());
    }
    let mut keep = KEEP.to_vec();
    if options.extended {
        keep.extend(["vinfo", "notes"]);
    }
    let directories = port_directories(&root)?;
    let results: Mutex<Vec<Option<Result<Entries>>>> =
        Mutex::new((0..directories.len()).map(|_| None).collect());
    let next = Mutex::new(0usize);
    let skipped = Mutex::new(0usize);
    let workers = std::thread::available_parallelism().map_or(4, usize::from);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let position = {
                        let mut next = next.lock().unwrap();
                        let position = *next;
                        *next += 1;
                        position
                    };
                    let Some(portdir) = directories.get(position) else {
                        break;
                    };
                    let portfile = root.join(portdir).join("Portfile");
                    let unchanged = fs::metadata(&portfile)
                        .and_then(|metadata| metadata.modified())
                        .is_ok_and(|modified| modified <= old_time);
                    let directory_name = portdir.rsplit('/').next().unwrap_or(portdir);
                    let reused = unchanged
                        .then(|| old.get(&directory_name.to_ascii_lowercase()))
                        .flatten()
                        .and_then(|(name, line)| {
                            let mut entries = vec![(name.clone(), line.clone())];
                            let subports = crate::tcl_list::dictionary(line)
                                .ok()
                                .and_then(|fields| fields.get("subports").cloned())
                                .and_then(|value| crate::tcl_list::parse(&value).ok())
                                .unwrap_or_default();
                            for subport in subports {
                                let found = old.get(&subport.to_ascii_lowercase())?;
                                entries.push(found.clone());
                            }
                            Some(entries)
                        });
                    let result = match reused {
                        Some(entries) => {
                            *skipped.lock().unwrap() += 1;
                            Ok(entries)
                        }
                        None => index_port(&root, portdir, &variables, environment, &keep),
                    };
                    results.lock().unwrap()[position] = Some(result);
                }
            });
        }
    });
    let mut stats = Stats {
        skipped: skipped.into_inner().unwrap(),
        ..Stats::default()
    };
    let staged = tempfile::Builder::new()
        .prefix("mports.portindex.")
        .tempfile_in(&output)?;
    let mut writer = std::io::BufWriter::new(staged.as_file());
    let mut quick = String::new();
    let mut offset = 0usize;
    for (portdir, result) in directories.iter().zip(results.into_inner().unwrap()) {
        stats.total += 1;
        match result.expect("every directory is indexed") {
            Ok(entries) => {
                for (name, line) in entries {
                    let text = format!(
                        "{name} {}\n{line}\n",
                        crate::index::record_length(&line) + 1
                    );
                    quick.push_str(&format!("{} {offset}\n", name.to_ascii_lowercase()));
                    offset += text.len();
                    writer.write_all(text.as_bytes())?;
                }
            }
            Err(err) => {
                stats.failed += 1;
                progress.error(&format!("Failed to parse file {portdir}/Portfile: {err:#}"));
            }
        }
    }
    writer.flush()?;
    drop(writer);
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(staged.path(), fs::Permissions::from_mode(0o644))?;
    staged.persist(&index_path)?;
    fs::write(output.join("PortIndex.quick"), quick)?;
    if !options.quiet {
        println!(
            "\nTotal number of ports parsed:\t{}\nPorts successfully parsed:\t{}\nPorts failed:\t\t\t{}\nUp-to-date ports skipped:\t{}\n",
            stats.total,
            stats.total - stats.failed,
            stats.failed,
            stats.skipped
        );
    }
    Ok(stats)
}

/// The `portindex` command line: `portindex [-d] [-e] [-f] [-o dir] [-p plat_ver_arch] [-q] [-x] [dir]`.
pub fn main(args: &[String]) -> Result<i32> {
    let mut options = Options::default();
    let mut directory = PathBuf::from(".");
    let mut permit_error = false;
    let mut arguments = args.iter();
    while let Some(arg) = arguments.next() {
        match arg.as_str() {
            "-d" => {}
            "-e" => permit_error = true,
            "-f" => options.full = true,
            "-q" => options.quiet = true,
            "-x" => options.extended = true,
            "-o" => {
                options.output = Some(PathBuf::from(
                    arguments.next().context("-o requires a directory")?,
                ))
            }
            "-p" => {
                options.platform = Some(arguments.next().context("-p requires a platform")?.clone())
            }
            flag if flag.starts_with('-') => {
                bail!(
                    "Unknown option: {flag}\nUsage: portindex [-d] [-e] [-f] [-o output directory] [-p plat_ver_arch] [-q] [-x] [directory]"
                )
            }
            path => directory = PathBuf::from(path),
        }
    }
    if !options.quiet {
        println!(
            "Creating port index in {}",
            options.output.as_deref().unwrap_or(&directory).display()
        );
    }
    let stats = run(&directory, &options)?;
    Ok(if permit_error && stats.failed > 0 {
        2
    } else {
        0
    })
}
