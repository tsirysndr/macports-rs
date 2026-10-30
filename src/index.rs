use crate::tcl_list;
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeMap, path::Path};

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
    pub ports: BTreeMap<String, Port>,
}
impl Index {
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
        Ok(Self { ports })
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
