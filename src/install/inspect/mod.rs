//! Read-only inspection of authenticated bzip2 tar archives. No extraction or Tcl.
mod contents;
use super::archive::VerifiedArchive;
use anyhow::{Context, Result, ensure};
pub use contents::Contents;
use std::{
    collections::{BTreeMap, HashSet},
    io::{self, BufReader, Read},
};

#[derive(Clone, Copy)]
pub struct Limits {
    pub expanded_bytes: u64,
    pub entries: usize,
    pub contents_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            expanded_bytes: 16 * 1024 * 1024 * 1024,
            entries: 200_000,
            contents_bytes: 16 * 1024 * 1024,
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Directory,
    Symlink(String),
    Hardlink(String),
}
#[derive(Debug)]
pub struct Member {
    pub kind: Kind,
    pub size: u64,
    pub mode: u32,
    pub data_offset: u64,
    pub attributes: BTreeMap<String, String>,
}
#[derive(Debug)]
pub struct Inspection {
    pub contents: Contents,
    pub members: BTreeMap<String, Member>,
}

pub fn tbz2(archive: &mut VerifiedArchive, limits: Limits) -> Result<Inspection> {
    ensure!(
        limits.expanded_bytes > 0 && limits.entries > 0 && limits.contents_bytes > 0,
        "archive inspection limits must be positive"
    );
    let decoder = bzip2::bufread::BzDecoder::new(BufReader::new(archive.reader()?));
    let mut tar = tar::Archive::new(Bounded {
        inner: decoder,
        remaining: limits.expanded_bytes,
    });
    let mut members = BTreeMap::new();
    let mut folded = HashSet::new();
    let mut contents = None;
    let mut pending_pax = None;
    // Raw mode lets us bound extension records before allocating them and reject
    // unsupported size/sparse overrides instead of silently misreading payloads.
    for (count, entry) in tar.entries()?.raw(true).enumerate() {
        ensure!(count < limits.entries, "archive entry limit exceeded");
        let mut entry = entry.context("invalid tar entry")?;
        let ty = entry.header().entry_type();
        if ty.is_pax_local_extensions() {
            ensure!(
                pending_pax.is_none(),
                "consecutive PAX headers are unsupported"
            );
            ensure!(entry.size() <= 64 * 1024, "PAX metadata exceeds size limit");
            let mut data = Vec::new();
            entry.read_to_end(&mut data)?;
            ensure!(
                data.last() == Some(&b'\n')
                    && !data[..data.len() - 1]
                        .split(|&b| b == b'\n')
                        .any(|line| line.is_empty()),
                "invalid PAX record framing"
            );
            let mut attributes = BTreeMap::new();
            for item in tar::PaxExtensions::new(&data) {
                let item = item?;
                let key = item.key()?;
                let value = item.value()?;
                ensure!(
                    matches!(
                        key,
                        "path"
                            | "linkpath"
                            | "mtime"
                            | "atime"
                            | "ctime"
                            | "uid"
                            | "gid"
                            | "uname"
                            | "gname"
                            | "SCHILY.fflags"
                    ),
                    "unsupported PAX field {key:?}"
                );
                ensure!(
                    attributes
                        .insert(key.to_owned(), value.to_owned())
                        .is_none(),
                    "duplicate PAX field"
                );
            }
            pending_pax = Some(attributes);
            continue;
        }
        let mut attributes = pending_pax.take().unwrap_or_default();
        let raw = entry.path_bytes();
        let name = path(
            attributes
                .get("path")
                .map(String::as_str)
                .unwrap_or(std::str::from_utf8(&raw).context("non-UTF-8 archive path")?),
        )?;
        let link_override = attributes.remove("linkpath");
        ensure!(
            link_override.is_none() || ty.is_symlink() || ty.is_hard_link(),
            "PAX linkpath on non-link entry"
        );
        let size = entry.size();
        let kind = if ty.is_file() {
            Kind::File
        } else if ty.is_dir() {
            Kind::Directory
        } else if ty.is_symlink() {
            let link = entry.link_name_bytes().context("missing symlink target")?;
            let target = link_override
                .clone()
                .unwrap_or(std::str::from_utf8(&link)?.to_owned());
            ensure!(
                !target.is_empty() && target.len() <= 4096 && !target.chars().any(char::is_control),
                "invalid symlink target"
            );
            Kind::Symlink(target)
        } else if ty.is_hard_link() {
            let link = entry.link_name_bytes().context("missing hardlink target")?;
            Kind::Hardlink(path(
                link_override
                    .as_deref()
                    .unwrap_or(std::str::from_utf8(&link)?),
            )?)
        } else {
            anyhow::bail!("unsupported tar entry type for {name:?}");
        };
        if kind != Kind::File {
            ensure!(size == 0, "non-file tar entry has payload data");
        }
        if name.is_empty() {
            ensure!(
                kind == Kind::Directory && !members.contains_key(""),
                "invalid archive root entry"
            );
        }
        ensure!(
            folded.insert(name.to_ascii_lowercase()),
            "duplicate or case-colliding archive path {name:?}"
        );
        if name.starts_with('+') {
            ensure!(
                !name.contains('/') && kind == Kind::File,
                "archive control files must be regular root files"
            );
            if name == "+CONTENTS" {
                ensure!(
                    size <= limits.contents_bytes,
                    "+CONTENTS exceeds size limit"
                );
                let mut text = String::new();
                entry
                    .read_to_string(&mut text)
                    .context("invalid +CONTENTS data")?;
                ensure!(text.len() as u64 == size, "truncated +CONTENTS");
                contents = Some(Contents::parse(&text)?);
            }
        }
        let mode = entry.header().mode()?;
        let data_offset = entry.raw_file_position();
        members.insert(
            name,
            Member {
                kind,
                size,
                mode,
                data_offset,
                attributes,
            },
        );
    }
    // Validate the bzip2 checksum/end marker and reject concealed tar payloads
    // after the terminator. A conventional tar terminator has two zero blocks.
    ensure!(pending_pax.is_none(), "PAX metadata has no following entry");
    let mut bounded = tar.into_inner();
    let mut padding = 0u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = bounded.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        ensure!(
            buffer[..n].iter().all(|&b| b == 0),
            "nonzero data after tar terminator"
        );
        padding += n as u64;
    }
    ensure!(padding >= 512, "missing tar end marker");
    let mut compressed = bounded.inner.into_inner();
    ensure!(
        compressed.read(&mut buffer[..1])? == 0,
        "trailing compressed data after archive"
    );
    let contents = contents.context("archive has no +CONTENTS")?;
    let by_folded: std::collections::HashMap<_, _> = members
        .iter()
        .map(|(name, member)| (name.to_ascii_lowercase(), member))
        .collect();
    let mut payload = std::collections::BTreeSet::new();
    for (name, member) in &members {
        let mut ancestor = name.as_str();
        while let Some((parent, _)) = ancestor.rsplit_once('/') {
            if let Some(member) = by_folded.get(&parent.to_ascii_lowercase()) {
                ensure!(
                    member.kind == Kind::Directory,
                    "archive path descends through a non-directory: {name:?}"
                );
            }
            ancestor = parent;
        }
        if let Kind::Hardlink(target) = &member.kind {
            ensure!(
                members.get(target).is_some_and(|m| m.kind == Kind::File)
                    && !target.starts_with('+'),
                "hardlink must target a payload regular file"
            );
        }
        if !name.is_empty() && !name.starts_with('+') && member.kind != Kind::Directory {
            payload.insert(name.clone());
        }
    }
    ensure!(
        payload == contents.files,
        "+CONTENTS inventory does not match archive payload"
    );
    Ok(Inspection { contents, members })
}
fn path(raw: &str) -> Result<String> {
    ensure!(
        raw.len() <= 4096
            && !raw.starts_with('/')
            && !raw.contains('\\')
            && !raw.chars().any(char::is_control),
        "invalid archive path"
    );
    let mut parts = Vec::new();
    for part in raw.split('/') {
        ensure!(part != "..", "archive path traversal is forbidden");
        if !matches!(part, "" | ".") {
            parts.push(part);
        }
    }
    Ok(parts.join("/"))
}
struct Bounded<R> {
    inner: R,
    remaining: u64,
}
impl<R: Read> Read for Bounded<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let limit = self.remaining.saturating_add(1).min(buffer.len() as u64) as usize;
        let n = self.inner.read(&mut buffer[..limit])?;
        if n as u64 > self.remaining {
            return Err(io::Error::other("expanded archive exceeds size limit"));
        }
        self.remaining -= n as u64;
        Ok(n)
    }
}
