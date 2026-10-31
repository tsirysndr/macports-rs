//! MacPorts line-based build state, held under an exclusive advisory lock.
//! This stores phase/variant records; lifecycle policy (including cleaning a
//! stale work tree) belongs to the caller, not to the state-file reader.
use anyhow::{Context, Result};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, Seek, SeekFrom, Write},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        io::AsRawFd,
    },
    path::Path,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockStatus {
    /// Upstream dry-run state handles do not acquire a lock.
    NotRequested,
    Exclusive,
    Unsupported,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Compatibility {
    Current,
    PortfileChanged,
    UnsupportedVersion(String),
}
#[derive(Debug)]
pub struct StateFile {
    file: File,
    pub lock_status: LockStatus,
}
impl StateFile {
    /// Open an existing state file (or /dev/null) without locking, as required
    /// by PortSystem dry-run opening. Never creates a file.
    pub fn open_unlocked(path: &Path) -> Result<Self> {
        Ok(Self {
            file: OpenOptions::new().read(true).append(true).open(path)?,
            lock_status: LockStatus::NotRequested,
        })
    }
    /// Open/create and wait for the exclusive lock. The parent must exist.
    pub fn open(path: &Path, creation_mask: Option<u32>) -> Result<Self> {
        Self::acquire(path, creation_mask, false)?.context("state lock unexpectedly unavailable")
    }
    /// Return None if another open file description holds the lock. Platforms
    /// without flock support return a handle marked Unsupported, as MacPorts
    /// permits continuing there. All other locking errors propagate.
    pub fn try_open(path: &Path, creation_mask: Option<u32>) -> Result<Option<Self>> {
        Self::acquire(path, creation_mask, true)
    }
    fn acquire(path: &Path, mask: Option<u32>, nonblocking: bool) -> Result<Option<Self>> {
        let mut options = OpenOptions::new();
        options.read(true).append(true);
        let file = match options
            .clone()
            .create_new(true)
            .mode(if mask.is_some() { 0o600 } else { 0o666 })
            .open(path)
        {
            Ok(file) => {
                if let Some(mask) = mask {
                    file.set_permissions(std::fs::Permissions::from_mode(0o666 & !mask))?;
                }
                file
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => options.open(path)?,
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("cannot open build state {}", path.display()));
            }
        };
        let operation = libc::LOCK_EX | if nonblocking { libc::LOCK_NB } else { 0 };
        let lock_status = loop {
            // SAFETY: the descriptor is owned and live for the syscall. Closing
            // this File releases the lock, including on early error paths.
            if unsafe { libc::flock(file.as_raw_fd(), operation) } == 0 {
                break LockStatus::Exclusive;
            }
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if nonblocking && err.kind() == io::ErrorKind::WouldBlock {
                return Ok(None);
            }
            if err.raw_os_error() == Some(libc::EOPNOTSUPP) {
                break LockStatus::Unsupported;
            }
            return Err(err).with_context(|| format!("cannot lock build state {}", path.display()));
        };
        Ok(Some(Self { file, lock_status }))
    }
    pub fn lines(&mut self) -> Result<Vec<String>> {
        self.file.seek(SeekFrom::Start(0))?;
        BufReader::new(&mut self.file)
            .lines()
            .collect::<io::Result<Vec<_>>>()
            .context("cannot read build state")
    }
    /// Get the first value for a literal class, including a match after a line
    /// prefix. Standard classes are version, checksum, variant, and target.
    pub fn first(&mut self, class: &str) -> Result<Option<String>> {
        let key = format!("{class}: ");
        Ok(self.lines()?.into_iter().find_map(|line| {
            line.find(&key)
                .map(|index| line[index + key.len()..].to_owned())
        }))
    }
    pub fn contains(&mut self, class: &str, id: &str) -> Result<bool> {
        let expected = format!("{class}: {id}");
        Ok(self.lines()?.iter().any(|line| line == &expected))
    }
    /// Append and flush only when this exact record is not already present.
    pub fn record(&mut self, class: &str, id: &str) -> Result<bool> {
        if self.contains(class, id)? {
            return Ok(false);
        }
        self.file.seek(SeekFrom::End(0))?;
        writeln!(self.file, "{class}: {id}")?;
        self.file.flush()?;
        Ok(true)
    }
    /// Rewrite fresh records without releasing the lock or replacing its inode.
    pub fn reset(&mut self, portfile_sha256: &str) -> Result<()> {
        self.file.set_len(0)?;
        self.initialize(portfile_sha256)
    }
    pub fn initialize(&mut self, portfile_sha256: &str) -> Result<()> {
        self.record("version", "2")?;
        self.record("checksum", portfile_sha256)?;
        Ok(())
    }
    /// Match upstream v1 timestamp and v2 checksum invalidation. Unsupported
    /// versions are reported to the caller rather than silently discarded.
    pub fn compatibility(
        &mut self,
        portfile_sha256: &str,
        portfile_mtime: i64,
    ) -> Result<Compatibility> {
        match self.first("version")?.as_deref().unwrap_or("1") {
            "1" => Ok(if self.file.metadata()?.mtime() < portfile_mtime {
                Compatibility::PortfileChanged
            } else {
                Compatibility::Current
            }),
            "2" => Ok(
                if self.first("checksum")?.as_deref() == Some(portfile_sha256) {
                    Compatibility::Current
                } else {
                    Compatibility::PortfileChanged
                },
            ),
            version => Ok(Compatibility::UnsupportedVersion(version.into())),
        }
    }
    /// Last duplicate variant wins. With no variant or target records, an
    /// untouched state file accepts any requested variant selection.
    pub fn variants_match(
        &mut self,
        requested: &BTreeMap<String, String>,
    ) -> Result<(bool, BTreeMap<String, String>)> {
        let (found, old) = self.variants()?;
        Ok((!found || old == *requested, old))
    }
    pub fn variants(&mut self) -> Result<(bool, BTreeMap<String, String>)> {
        let mut old = BTreeMap::new();
        let mut targets = false;
        for line in self.lines()? {
            if let Some(index) = line.find("variant: ") {
                let value = &line[index + 9..];
                let end = value.chars().next().map(char::len_utf8).unwrap_or(0);
                old.insert(value[end..].into(), value[..end].into());
            }
            targets |= line.contains("target: ");
        }
        Ok((targets || !old.is_empty(), old))
    }
}
