use crate::tcl_list;
use anyhow::{Context, Result, ensure};
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::OnceLock,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    pub name: String,
    pub fields: BTreeMap<String, String>,
}
impl Port {
    pub fn get(&self, field: &str) -> &str {
        self.fields.get(field).map(String::as_str).unwrap_or("")
    }
    pub fn version(&self) -> String {
        let revision = self.get("revision");
        format!(
            "{}_{}",
            self.get("version"),
            if revision.is_empty() { "0" } else { revision }
        )
    }
}
/// Ports evaluated from directories the user named outside the ports tree
/// (`port info ./myport`), by lowercase name. They live for the process.
static EXTERNAL: std::sync::Mutex<Option<HashMap<String, &'static Port>>> =
    std::sync::Mutex::new(None);

/// Directories the user named explicitly; only these may hold an absolute
/// `portdir` outside the selected tree.
/// Each directory comes with PortGroup directories searched before the tree's.
#[allow(clippy::type_complexity)]
static EXTERNAL_DIRECTORIES: std::sync::Mutex<Vec<(std::path::PathBuf, Vec<std::path::PathBuf>)>> =
    std::sync::Mutex::new(Vec::new());

/// Allow Portfiles from `directory`, which the user named on the command line.
pub fn allow_external_directory(directory: &Path) {
    allow_external_directory_with_groups(directory, Vec::new());
}

/// Allow a Portfile saved in the registry, with the PortGroups saved beside it.
pub fn allow_external_directory_with_groups(directory: &Path, groups: Vec<std::path::PathBuf>) {
    let mut directories = EXTERNAL_DIRECTORIES.lock().expect("external directories");
    match directories.iter_mut().find(|(known, _)| known == directory) {
        Some(entry) => entry.1 = groups,
        None => directories.push((directory.to_path_buf(), groups)),
    }
}

pub fn is_external_directory(directory: &Path) -> bool {
    external_groups(directory).is_some()
}

pub fn external_groups(directory: &Path) -> Option<Vec<std::path::PathBuf>> {
    EXTERNAL_DIRECTORIES
        .lock()
        .expect("external directories")
        .iter()
        .find(|(known, _)| known == directory)
        .map(|(_, groups)| groups.clone())
}

/// Make an evaluated out-of-tree port visible to every catalog lookup.
pub fn add_external_port(port: Port) {
    let leaked: &'static Port = Box::leak(Box::new(port));
    EXTERNAL
        .lock()
        .expect("external ports")
        .get_or_insert_with(HashMap::new)
        .insert(leaked.name.to_ascii_lowercase(), leaked);
}

fn external_port(name: &str) -> Option<&'static Port> {
    EXTERNAL
        .lock()
        .expect("external ports")
        .as_ref()?
        .get(&name.to_ascii_lowercase())
        .copied()
}

#[derive(Debug)]
pub struct Index {
    ports: BTreeMap<String, Port>,
    folded_names: OnceLock<HashMap<String, String>>,
}
/// Tcl 8.6 string length, which PortIndex headers use: UTF-16 code units.
pub fn record_length(record: &str) -> usize {
    record.encode_utf16().count()
}

/// The byte offset in `body` after a record `length` UTF-16 units long.
pub fn record_end(body: &str, length: usize) -> Option<usize> {
    // ASCII records (nearly all) have one unit per byte.
    if body.len() >= length && body.as_bytes()[..length].is_ascii() {
        return Some(length);
    }
    let mut units = 0;
    for (offset, character) in body.char_indices() {
        if units == length {
            return Some(offset);
        }
        units += character.len_utf16();
    }
    (units == length).then_some(body.len())
}

/// One record's fields, checked against the name in its header.
fn record((name, body): &(String, &str)) -> Result<Port> {
    let values = tcl_list::parse(body).with_context(|| format!("invalid fields for {name}"))?;
    ensure!(values.len() % 2 == 0, "odd field count for {name}");
    let mut values = values.into_iter();
    let mut fields = BTreeMap::new();
    while let (Some(key), Some(value)) = (values.next(), values.next()) {
        fields.insert(key, value);
    }
    ensure!(
        fields.get("name") == Some(name),
        "record name mismatch for {name}"
    );
    Ok(Port {
        name: name.clone(),
        fields,
    })
}

impl Index {
    /// Catalogs are immutable after parsing, keeping lazy lookup data consistent.
    pub fn ports(&self) -> &BTreeMap<String, Port> {
        &self.ports
    }

    /// Exact spelling takes priority. Fallback retains the original ASCII case
    /// comparison and deterministic first-name behavior for case collisions.
    pub fn lookup(&self, name: &str) -> Option<&Port> {
        if let Some(port) = self.ports.get(name) {
            return Some(port);
        }
        let names = self.folded_names.get_or_init(|| {
            let mut names = HashMap::with_capacity(self.ports.len());
            for name in self.ports.keys() {
                names
                    .entry(name.to_ascii_lowercase())
                    .or_insert_with(|| name.clone());
            }
            names
        });
        names
            .get(&name.to_ascii_lowercase())
            .and_then(|canonical| self.ports.get(canonical))
            .or_else(|| external_port(name))
    }

    pub fn read(path: &Path) -> Result<Self> {
        Self::parse(&std::fs::read_to_string(path).with_context(|| {
            format!(
                "cannot read index {} (use --index-file or PORT_RS_INDEX)",
                path.display()
            )
        })?)
    }
    pub fn parse(input: &str) -> Result<Self> {
        let mut rest = input;
        let mut records = Vec::new();
        while !rest.is_empty() {
            let (header, body) = rest
                .split_once('\n')
                .context("truncated PortIndex header")?;
            let header = tcl_list::parse(header)?;
            ensure!(header.len() == 2, "invalid PortIndex header");
            let length: usize = header[1]
                .parse()
                .context("invalid PortIndex record length")?;
            ensure!(length > 0, "empty PortIndex record");
            let end = record_end(body, length)
                .filter(|end| body[..*end].ends_with('\n'))
                .with_context(|| format!("truncated or invalid record for {}", header[0]))?;
            let [name, _] = <[String; 2]>::try_from(header).unwrap();
            records.push((name, &body[..end - 1]));
            rest = &body[end..];
        }
        // Field lists are independent; parse them on every core.
        let threads = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .min(records.len().div_ceil(2048))
            .max(1);
        let chunk = records.len().div_ceil(threads).max(1);
        let parsed: Vec<Result<Vec<Port>>> = std::thread::scope(|scope| {
            let workers: Vec<_> = records
                .chunks(chunk)
                .map(|records| scope.spawn(move || records.iter().map(record).collect()))
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().expect("PortIndex parser panicked"))
                .collect()
        });
        let mut ports = BTreeMap::new();
        for chunk in parsed {
            for port in chunk? {
                ensure!(
                    !ports.contains_key(&port.name),
                    "duplicate port {}",
                    port.name
                );
                ports.insert(port.name.clone(), port);
            }
        }
        Ok(Self {
            ports,
            folded_names: OnceLock::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(name: &str, fields: &str) -> String {
        format!("{name} {}\n{fields}\n", fields.encode_utf16().count() + 1)
    }
    #[test]
    fn unicode_multiline() {
        let s = record(
            "hello",
            "name hello version 1.0 description {café 😀\nsecond line}",
        );
        let index = Index::parse(&s).unwrap();
        assert_eq!(
            index.ports["hello"].get("description"),
            "café 😀\nsecond line"
        );
        assert_eq!(index.ports["hello"].version(), "1.0_0");
    }
    #[test]
    fn rejects_corruption() {
        for s in [
            "x 200\nname x\n".to_owned(),
            record("x", "name y"),
            record("x", "name x version"),
            record("x", "name x").repeat(2),
        ] {
            assert!(Index::parse(&s).is_err());
        }
    }
}

#[cfg(test)]
mod lookup_tests {
    use super::*;
    fn catalog(names: &[&str]) -> Index {
        let mut data = String::new();
        for name in names {
            let body = format!("name {name} version 1");
            data.push_str(&format!(
                "{name} {}\n{body}\n",
                body.encode_utf16().count() + 1
            ));
        }
        Index::parse(&data).unwrap()
    }
    #[test]
    fn exact_names_win_and_folded_collisions_are_deterministic() {
        let index = catalog(&["foo", "Foo", "FOO", "Bar"]);
        for name in ["foo", "Foo", "FOO", "Bar"] {
            assert_eq!(index.lookup(name).unwrap().name, name);
        }
        assert_eq!(index.lookup("fOo").unwrap().name, "FOO");
        assert_eq!(index.lookup("bar").unwrap().name, "Bar");
        assert!(index.lookup("missing").is_none());
        assert_eq!(index.lookup("foo").unwrap().name, "foo");
    }
    #[test]
    fn lookup_preserves_non_ascii_case_and_empty_catalog_behavior() {
        let index = catalog(&["Éclair"]);
        assert_eq!(index.lookup("ÉCLAIR").unwrap().name, "Éclair");
        assert!(index.lookup("éclair").is_none());
        assert!(catalog(&[]).lookup("anything").is_none());
    }
    #[test]
    fn shared_catalog_supports_concurrent_first_lookups() {
        let index = catalog(&["curl", "zlib"]);
        std::thread::scope(|scope| {
            let threads: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        assert_eq!(index.lookup("CURL").unwrap().name, "curl");
                        assert!(index.lookup("missing").is_none());
                    })
                })
                .collect();
            for thread in threads {
                thread.join().unwrap();
            }
        });
    }
}
