//! Pextlib's `filemap` database: a tree of path components with string
//! values at the leaves, stored in the `org.darwinports.filemap` format that
//! MacPorts base reads and writes (`distfiles_mirror.db`).
use anyhow::{Context, Result, bail, ensure};
use std::{cmp::Ordering, fs, path::Path};

const SIGNATURE: &[u8; 24] = b"org.darwinports.filemap\0";
const VERSION: [u8; 4] = [0, 1, 0, 0];
const NAME_MAX: usize = 255;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Directory(String, Vec<Node>),
    Leaf(String, String),
}

impl Node {
    fn key(&self) -> &str {
        match self {
            Node::Directory(key, _) | Node::Leaf(key, _) => key,
        }
    }
}

/// `strncasecmp(existing, part, part.len())`, then a longer existing key sorts
/// after: the order filemap keeps sibling components in.
fn compare(existing: &str, part: &str) -> Ordering {
    let length = part.len().min(existing.len());
    let prefix = existing.as_bytes()[..length]
        .iter()
        .map(u8::to_ascii_lowercase)
        .cmp(part.as_bytes()[..length].iter().map(u8::to_ascii_lowercase));
    prefix.then(existing.len().cmp(&part.len()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filemap {
    root: Vec<Node>,
}

impl Default for Filemap {
    fn default() -> Self {
        Self::new()
    }
}

impl Filemap {
    pub fn new() -> Self {
        Self { root: Vec::new() }
    }

    /// Read a filemap; a missing or empty file is an empty map.
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                return Err(error).with_context(|| format!("cannot read {}", path.display()));
            }
        };
        if bytes.is_empty() {
            return Ok(Self::new());
        }
        ensure!(
            bytes.len() >= SIGNATURE.len() + VERSION.len(),
            "{} is not a filemap",
            path.display()
        );
        ensure!(
            &bytes[..SIGNATURE.len()] == SIGNATURE,
            "{}: filemap signature mismatch",
            path.display()
        );
        ensure!(
            bytes[SIGNATURE.len()..SIGNATURE.len() + 4] == VERSION,
            "{}: unknown filemap version",
            path.display()
        );
        let mut cursor = &bytes[SIGNATURE.len() + 4..];
        match read_node(&mut cursor)? {
            Node::Directory(_, children) => Ok(Self { root: children }),
            Node::Leaf(..) => bail!("{}: filemap root is not a directory", path.display()),
        }
    }

    /// Write atomically through `<path>.w`, as Pextlib does.
    pub fn save(&self, path: &Path) -> Result<()> {
        let mut bytes = SIGNATURE.to_vec();
        bytes.extend(VERSION);
        write_node(
            &mut bytes,
            &Node::Directory(String::new(), self.root.clone()),
        )?;
        let temporary = format!("{}.w", path.display());
        fs::write(&temporary, &bytes).with_context(|| format!("cannot write {temporary}"))?;
        fs::rename(&temporary, path).with_context(|| format!("cannot replace {}", path.display()))
    }

    /// Map a path to a value, creating directories; an existing directory at
    /// the leaf, or a file in the middle of the path, is an error.
    pub fn set(&mut self, path: &str, value: &str) -> Result<()> {
        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        ensure!(!parts.is_empty(), "filemap: empty path");
        ensure!(
            value.len() <= NAME_MAX && parts.iter().all(|part| part.len() <= NAME_MAX),
            "filemap: name too long"
        );
        let mut children = &mut self.root;
        for (index, part) in parts.iter().enumerate() {
            let last = index == parts.len() - 1;
            let position = match children
                .iter()
                .position(|node| compare(node.key(), part) != Ordering::Less)
            {
                Some(position) if compare(children[position].key(), part) == Ordering::Equal => {
                    position
                }
                found => {
                    let position = found.unwrap_or(children.len());
                    let node = if last {
                        Node::Leaf(part.to_string(), String::new())
                    } else {
                        Node::Directory(part.to_string(), Vec::new())
                    };
                    children.insert(position, node);
                    position
                }
            };
            match &mut children[position] {
                Node::Leaf(_, stored) if last => {
                    *stored = value.to_string();
                    return Ok(());
                }
                Node::Leaf(..) => bail!("filemap: {path}: not a directory"),
                Node::Directory(..) if last => bail!("filemap: {path}: is a directory"),
                Node::Directory(_, next) => children = next,
            }
        }
        Ok(())
    }

    /// The value for a path, matching components case-insensitively.
    pub fn get(&self, path: &str) -> Option<&str> {
        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        let mut children = &self.root;
        for (index, part) in parts.iter().enumerate() {
            let node = children
                .iter()
                .find(|node| compare(node.key(), part) == Ordering::Equal)?;
            match node {
                Node::Leaf(_, value) if index == parts.len() - 1 => return Some(value),
                Node::Directory(_, next) if index < parts.len() - 1 => children = next,
                _ => return None,
            }
        }
        None
    }

    /// Every mapped path with its value, depth first.
    pub fn entries(&self) -> Vec<(String, String)> {
        fn walk(prefix: &str, nodes: &[Node], out: &mut Vec<(String, String)>) {
            for node in nodes {
                match node {
                    Node::Leaf(key, value) => out.push((format!("{prefix}/{key}"), value.clone())),
                    Node::Directory(key, children) => {
                        walk(&format!("{prefix}/{key}"), children, out)
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk("", &self.root, &mut out);
        out
    }
}

fn take_string(cursor: &mut &[u8]) -> Result<String> {
    let end = cursor
        .iter()
        .position(|byte| *byte == 0)
        .context("filemap: unexpected end of data")?;
    ensure!(end <= NAME_MAX, "filemap: name too long");
    let text = String::from_utf8_lossy(&cursor[..end]).into_owned();
    *cursor = &cursor[end + 1..];
    Ok(text)
}

fn read_node(cursor: &mut &[u8]) -> Result<Node> {
    let (&kind, rest) = cursor
        .split_first()
        .context("filemap: unexpected end of data")?;
    *cursor = rest;
    let key = take_string(cursor)?;
    match kind {
        1 => Ok(Node::Leaf(key, take_string(cursor)?)),
        0 => {
            ensure!(cursor.len() >= 4, "filemap: unexpected end of data");
            let count = u32::from_be_bytes(cursor[..4].try_into().expect("four bytes"));
            *cursor = &cursor[4..];
            let children = (0..count)
                .map(|_| read_node(cursor))
                .collect::<Result<Vec<_>>>()?;
            Ok(Node::Directory(key, children))
        }
        other => bail!("filemap: unknown node kind {other}"),
    }
}

fn write_node(out: &mut Vec<u8>, node: &Node) -> Result<()> {
    match node {
        Node::Leaf(key, value) => {
            out.push(1);
            out.extend(key.as_bytes());
            out.push(0);
            out.extend(value.as_bytes());
            out.push(0);
        }
        Node::Directory(key, children) => {
            out.push(0);
            out.extend(key.as_bytes());
            out.push(0);
            out.extend(u32::try_from(children.len())?.to_be_bytes());
            for child in children {
                write_node(out, child)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_in_pextlib_order_and_layout() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("map.db");
        let mut map = Filemap::new();
        map.set(
            "/opt/local/var/macports/distfiles/zlib/zlib-1.3.tar.xz",
            "1",
        )
        .unwrap();
        map.set("/opt/local/var/macports/distfiles/Curl/curl.tar.gz", "1")
            .unwrap();
        map.set("/opt/local/var/macports/distfiles/apr/apr.tar.gz", "1")
            .unwrap();
        map.save(&path).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"org.darwinports.filemap\0\0\x01\0\0"));
        let loaded = Filemap::load(&path).unwrap();
        assert_eq!(loaded, map);
        let names: Vec<_> = loaded
            .entries()
            .into_iter()
            .map(|(path, _)| path.rsplit('/').nth(1).unwrap().to_string())
            .collect();
        assert_eq!(names, ["apr", "Curl", "zlib"]);
        assert_eq!(
            loaded.get("/OPT/local/var/macports/distfiles/curl/curl.tar.gz"),
            Some("1")
        );
        assert!(map.set("/opt/local/var", "1").is_err());
        assert_eq!(
            Filemap::load(&directory.path().join("missing")).unwrap(),
            Filemap::new()
        );
    }
}
