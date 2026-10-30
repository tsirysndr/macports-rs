use crate::tcl_list;
use anyhow::{Context, Result, ensure};
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::OnceLock,
};

#[derive(Debug)]
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
#[derive(Debug)]
pub struct Index {
    ports: BTreeMap<String, Port>,
    folded_names: OnceLock<HashMap<String, String>>,
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
        let mut ports = BTreeMap::new();
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
            // Tcl 8.6 string length counts UTF-16 code units, not UTF-8 bytes.
            let mut units = 0;
            let mut end = 0;
            for (offset, c) in body.char_indices() {
                units += c.len_utf16();
                end = offset + c.len_utf8();
                if units >= length {
                    break;
                }
            }
            ensure!(
                units == length && body[..end].ends_with('\n'),
                "truncated or invalid record for {}",
                header[0]
            );
            let values = tcl_list::parse(&body[..end - 1])
                .with_context(|| format!("invalid fields for {}", header[0]))?;
            ensure!(values.len() % 2 == 0, "odd field count for {}", header[0]);
            let fields: BTreeMap<_, _> = values
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| (p[0].clone(), p[1].clone()))
                .collect();
            ensure!(
                fields.get("name") == Some(&header[0]),
                "record name mismatch for {}",
                header[0]
            );
            let name = header[0].clone();
            ensure!(!ports.contains_key(&name), "duplicate port {name}");
            ports.insert(name.clone(), Port { name, fields });
            rest = &body[end..];
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
