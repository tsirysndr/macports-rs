//! Descriptor-relative, non-overwriting regular-file publication.
//! A coordinator must exclusively manage the destination directories while using
//! these primitives. Receipts provide in-process rollback, not crash recovery.
use anyhow::{Context, Result, ensure};
use std::{
    ffi::CString,
    fs::{File, OpenOptions},
    io::{self, Read},
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        io::{AsRawFd, FromRawFd},
    },
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

pub struct Root {
    directory: File,
}
impl Root {
    pub fn open(path: &Path) -> Result<Self> {
        let path: PathBuf = path.components().collect();
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        Ok(Self { directory })
    }
    /// Publish complete bytes and mode atomically without replacing an existing
    /// destination. Parents must already exist as physical directories. Dropping
    /// the returned receipt removes this file unless `persist` was called.
    pub fn publish_file(
        &self,
        relative: &Path,
        source: &mut impl Read,
        mode: u32,
    ) -> Result<PublishedFile> {
        ensure!(mode <= 0o7777, "invalid publication mode");
        let (parent, name) = self.parent(relative)?;
        let (file, temporary) = temporary(&parent)?;
        let mut output = file.as_ref();
        io::copy(source, &mut output).context("cannot copy publication payload")?;
        file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        file.sync_all()?;
        let metadata = file.metadata()?;
        // SAFETY: both descriptors and NUL-terminated names remain live. linkat
        // creates a new name atomically and never replaces an existing entry.
        if unsafe {
            libc::linkat(
                parent.as_raw_fd(),
                temporary.name.as_ptr(),
                parent.as_raw_fd(),
                name.as_ptr(),
                0,
            )
        } != 0
        {
            return Err(io::Error::last_os_error())
                .context("cannot publish file without overwriting");
        }
        let published = PublishedFile {
            parent,
            name,
            device: metadata.dev(),
            inode: metadata.ino(),
            retained: false,
            _anchor: file,
        };
        temporary.rollback()?;
        published.parent.sync_all()?;
        Ok(published)
    }
    fn parent(&self, path: &Path) -> Result<(File, CString)> {
        ensure!(
            !path.as_os_str().is_empty()
                && path.components().all(|c| matches!(c, Component::Normal(_))),
            "publication path must be relative and contain no parent components"
        );
        let mut components: Vec<_> = path.components().collect();
        let name = CString::new(components.pop().unwrap().as_os_str().as_bytes())?;
        let mut parent = self.directory.try_clone()?;
        for component in components {
            let name = CString::new(component.as_os_str().as_bytes())?;
            // SAFETY: parent owns the descriptor and name is NUL-terminated.
            let fd = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error())
                    .context("cannot open physical publication parent");
            }
            // SAFETY: openat returned a new owned descriptor.
            parent = unsafe { File::from_raw_fd(fd) };
        }
        Ok((parent, name))
    }
}

/// A published inode held pending registry commit. Rollback refuses to unlink a
/// replacement inode. Explicit rollback reports failures; Drop is best effort.
pub struct PublishedFile {
    parent: File,
    name: CString,
    device: u64,
    inode: u64,
    retained: bool,
    // Hold the inode open so its number cannot be recycled before rollback.
    _anchor: Arc<File>,
}
impl PublishedFile {
    /// Retain the published file after the coordinating transaction commits.
    pub fn persist(mut self) {
        self.retained = true;
    }
    pub fn rollback(mut self) -> Result<()> {
        let result = self.remove();
        self.retained = true;
        result
    }
    fn remove(&self) -> Result<()> {
        // SAFETY: stat has the correct layout and fstatat initializes it; the
        // parent descriptor and NUL-terminated leaf name remain live.
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::fstatat(
                self.parent.as_raw_fd(),
                self.name.as_ptr(),
                &mut stat,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::NotFound {
                return Ok(());
            }
            return Err(error.into());
        }
        ensure!(
            stat.st_dev as u64 == self.device && stat.st_ino as u64 == self.inode,
            "refusing to remove a replaced publication inode"
        );
        // SAFETY: unlink only the verified leaf entry, without following links.
        if unsafe { libc::unlinkat(self.parent.as_raw_fd(), self.name.as_ptr(), 0) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        self.parent.sync_all()?;
        Ok(())
    }
}
impl Drop for PublishedFile {
    fn drop(&mut self) {
        if !self.retained {
            let _ = self.remove();
        }
    }
}
fn temporary(parent: &File) -> Result<(Arc<File>, PublishedFile)> {
    let parent = parent.try_clone()?;
    for _ in 0..128 {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let name = CString::new(format!(
            ".port-rs-publish-{}-{sequence}",
            std::process::id()
        ))?;
        // SAFETY: the descriptor/name are valid; O_EXCL forbids following or
        // replacing any existing temporary entry, including symlinks.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600 as libc::c_uint,
            )
        };
        if fd < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::AlreadyExists {
                continue;
            }
            return Err(error.into());
        }
        // SAFETY: fd is a newly owned descriptor returned by openat.
        let file = Arc::new(unsafe { File::from_raw_fd(fd) });
        let metadata = match file.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                // SAFETY: this leaf was just exclusively created by us.
                unsafe {
                    libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0);
                }
                return Err(error.into());
            }
        };
        let temporary = PublishedFile {
            parent,
            name,
            device: metadata.dev(),
            inode: metadata.ino(),
            retained: false,
            _anchor: Arc::clone(&file),
        };
        return Ok((file, temporary));
    }
    anyhow::bail!("could not allocate a private publication filename")
}
