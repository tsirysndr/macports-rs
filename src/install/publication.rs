//! Descriptor-relative publication with in-process rollback receipts.
//! A coordinator must exclusively manage the destination directories while using
//! these primitives. Receipts provide in-process rollback, not crash recovery.
use super::inspect::Timestamp;
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
    /// Read a physical regular file through held parent descriptors. O_NONBLOCK
    /// prevents a substituted FIFO from blocking before its type is checked.
    pub(crate) fn open_file(&self, relative: &Path) -> Result<File> {
        let (parent, name) = self.parent(relative)?;
        // SAFETY: parent and the NUL-terminated name remain live for openat.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error()).context("cannot open stored file");
        }
        // SAFETY: openat returned a newly owned descriptor.
        let file = unsafe { File::from_raw_fd(fd) };
        ensure!(
            file.metadata()?.is_file(),
            "stored entry is not a regular file"
        );
        Ok(file)
    }
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
            directory: false,
            _anchor: file,
        };
        temporary.rollback()?;
        published.parent.sync_all()?;
        Ok(published)
    }
    /// Publish another name for an inode still held by a publication receipt.
    /// The source name must still refer to that inode. Both parents remain
    /// descriptor-relative, and an existing destination is never replaced.
    pub fn publish_hardlink(
        &self,
        relative: &Path,
        source: &PublishedFile,
    ) -> Result<PublishedFile> {
        let (parent, name) = self.parent(relative)?;
        // SAFETY: source owns its parent descriptor and NUL-terminated name.
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::fstatat(
                source.parent.as_raw_fd(),
                source.name.as_ptr(),
                &mut stat,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(io::Error::last_os_error()).context("cannot inspect hardlink source");
        }
        ensure!(
            stat.st_dev as u64 == source.device && stat.st_ino as u64 == source.inode,
            "hardlink source no longer matches its publication receipt"
        );
        ensure!(
            source._anchor.metadata()?.is_file(),
            "hardlink source must be a regular file"
        );
        // SAFETY: both parent descriptors and names remain live; no flags means
        // linkat does not follow the source leaf if it is replaced by a symlink.
        // The caller's exclusive directory access covers verification/publication.
        if unsafe {
            libc::linkat(
                source.parent.as_raw_fd(),
                source.name.as_ptr(),
                parent.as_raw_fd(),
                name.as_ptr(),
                0,
            )
        } != 0
        {
            return Err(io::Error::last_os_error())
                .context("cannot publish hardlink without overwriting");
        }
        let published = PublishedFile {
            parent,
            name,
            device: source.device,
            inode: source.inode,
            retained: false,
            directory: false,
            _anchor: Arc::clone(&source._anchor),
        };
        published.parent.sync_all()?;
        Ok(published)
    }
    /// Create one directory with explicit permissions, or share an existing
    /// physical directory without changing it. Parents must already exist.
    /// Roll back children before their parent: nonempty directories are retained.
    pub fn create_directory(
        &self,
        relative: &Path,
        mode: u32,
    ) -> Result<Option<PublishedDirectory>> {
        ensure!(mode <= 0o7777, "invalid directory publication mode");
        let (parent, name) = self.parent(relative)?;
        // SAFETY: descriptor and NUL-terminated leaf name are valid.
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::AlreadyExists {
                return Err(error.into());
            }
            // SAFETY: fstatat initializes stat and does not follow the leaf link.
            let mut stat: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe {
                libc::fstatat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    &mut stat,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                return Err(io::Error::last_os_error().into());
            }
            ensure!(
                stat.st_mode & libc::S_IFMT == libc::S_IFDIR,
                "existing publication entry is not a physical directory"
            );
            return Ok(None);
        }
        let opened = (|| -> Result<File> {
            // A restrictive inherited process mask can remove owner access from
            // mkdirat's mode. Restore private access before opening the inode.
            // The coordinator's exclusive directory ownership covers this step.
            // SAFETY: this is the directory leaf just created under parent.
            if unsafe { libc::fchmodat(parent.as_raw_fd(), name.as_ptr(), 0o700, 0) } != 0 {
                return Err(io::Error::last_os_error().into());
            }
            // SAFETY: open a new owned descriptor, rejecting a replaced link.
            let fd = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error().into());
            }
            // SAFETY: fd is the new owned descriptor returned above.
            Ok(unsafe { File::from_raw_fd(fd) })
        })();
        let file = match opened {
            Ok(file) => Arc::new(file),
            Err(error) => {
                // SAFETY: only remove our empty directory, never its descendants.
                unsafe {
                    libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR);
                }
                return Err(error);
            }
        };
        let metadata = match file.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                // SAFETY: this leaf belongs to the failed creation operation.
                unsafe {
                    libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR);
                }
                return Err(error.into());
            }
        };
        let entry = PublishedFile {
            parent,
            name,
            device: metadata.dev(),
            inode: metadata.ino(),
            retained: false,
            directory: true,
            _anchor: file,
        };
        entry
            ._anchor
            .set_permissions(std::fs::Permissions::from_mode(mode))?;
        entry._anchor.sync_all()?;
        entry.parent.sync_all()?;
        Ok(Some(PublishedDirectory { entry }))
    }
    /// Publish a symlink without interpreting its target or replacing an entry.
    /// macOS O_SYMLINK anchors the link inode itself, including dangling links.
    #[cfg(target_os = "macos")]
    pub fn publish_symlink(&self, relative: &Path, target: &Path) -> Result<PublishedFile> {
        let target = CString::new(target.as_os_str().as_bytes())?;
        let (parent, name) = self.parent(relative)?;
        // SAFETY: both names are NUL-terminated and parent owns its descriptor.
        // symlinkat creates the leaf exclusively and does not traverse target.
        if unsafe { libc::symlinkat(target.as_ptr(), parent.as_raw_fd(), name.as_ptr()) } != 0 {
            return Err(io::Error::last_os_error())
                .context("cannot publish symlink without overwriting");
        }
        let opened = (|| -> Result<Arc<File>> {
            // SAFETY: O_SYMLINK opens the symlink itself (Darwin open(2)), not
            // its target. Parent/name remain valid throughout the call.
            let fd = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_SYMLINK | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error().into());
            }
            // SAFETY: fd is a new owned descriptor returned by openat.
            let file = Arc::new(unsafe { File::from_raw_fd(fd) });
            ensure!(
                file.metadata()?.file_type().is_symlink(),
                "publication entry is no longer a symlink"
            );
            Ok(file)
        })();
        let file = match opened {
            Ok(file) => file,
            Err(error) => {
                // SAFETY: remove only the leaf created by this operation. The
                // caller exclusively manages this directory; targets are not followed.
                unsafe {
                    libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0);
                }
                return Err(error);
            }
        };
        let metadata = match file.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                // SAFETY: this is our newly created symlink leaf.
                unsafe {
                    libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0);
                }
                return Err(error.into());
            }
        };
        let published = PublishedFile {
            parent,
            name,
            device: metadata.dev(),
            inode: metadata.ino(),
            retained: false,
            directory: false,
            _anchor: file,
        };
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

/// Receipt for a newly created directory. Shared existing directories never
/// receive a receipt and are never modified by this API.
pub struct PublishedDirectory {
    entry: PublishedFile,
}
impl PublishedDirectory {
    pub(crate) fn prepare_rollback(&self) -> Result<()> {
        self.set_mode(0o700)
    }
    pub(crate) fn set_mode(&self, mode: u32) -> Result<()> {
        ensure!(mode <= 0o7777, "invalid publication mode");
        self.entry
            ._anchor
            .set_permissions(std::fs::Permissions::from_mode(mode))?;
        Ok(())
    }
    /// Restore final ownership and permissions after publishing children.
    pub fn set_owner_mode(&self, uid: u64, gid: u64, mode: u32) -> Result<()> {
        self.entry.set_owner_mode(uid, gid, mode)
    }
    /// Restore directory times after publishing all children.
    pub fn set_times(&self, atime: Option<Timestamp>, mtime: Timestamp) -> Result<()> {
        self.entry.set_times(atime, mtime)
    }
    pub fn persist(self) {
        self.entry.persist();
    }
    /// Remove only this directory, and only if it is still ours and empty.
    pub fn rollback(self) -> Result<()> {
        self.entry.rollback()
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
    directory: bool,
    // Hold the inode open so its number cannot be recycled before rollback.
    _anchor: Arc<File>,
}
impl PublishedFile {
    /// Restore numeric ownership before permissions: chown can clear set-ID bits.
    /// Both operations address only the held inode (the link itself for macOS
    /// symlink receipts). Requires OS permission; this does not elevate privileges.
    /// If chmod fails after chown, the caller must roll back the publication.
    pub fn set_owner_mode(&self, uid: u64, gid: u64, mode: u32) -> Result<()> {
        // All-ones is chown's "leave unchanged" sentinel, not an archive owner.
        ensure!(
            uid < u64::from(u32::MAX),
            "archive uid cannot be represented as an owner"
        );
        ensure!(
            gid < u64::from(u32::MAX),
            "archive gid cannot be represented as a group"
        );
        ensure!(mode <= 0o7777, "invalid publication mode");
        // SAFETY: the receipt owns this descriptor; IDs were checked above.
        if unsafe {
            libc::fchown(
                self._anchor.as_raw_fd(),
                uid as libc::uid_t,
                gid as libc::gid_t,
            )
        } != 0
        {
            return Err(io::Error::last_os_error()).context("cannot restore publication ownership");
        }
        self._anchor
            .set_permissions(std::fs::Permissions::from_mode(mode))
            .context("cannot restore publication permissions")
    }
    /// Set times on the held inode, including the link itself for macOS symlink
    /// receipts. An absent access time leaves the existing access time unchanged.
    /// Hardlink aliases share these times. No pathname is resolved by this call.
    pub fn set_times(&self, atime: Option<Timestamp>, mtime: Timestamp) -> Result<()> {
        let convert = |time: Timestamp| -> Result<libc::timespec> {
            ensure!(
                time.nanoseconds < 1_000_000_000,
                "invalid timestamp nanoseconds"
            );
            Ok(libc::timespec {
                tv_sec: time.seconds,
                tv_nsec: time.nanoseconds.into(),
            })
        };
        let times = [
            match atime {
                Some(time) => convert(time)?,
                None => libc::timespec {
                    tv_sec: 0,
                    tv_nsec: libc::UTIME_OMIT,
                },
            },
            convert(mtime)?,
        ];
        // SAFETY: the receipt owns the inode descriptor and times contains two
        // initialized timespec values for the duration of the syscall.
        if unsafe { libc::futimens(self._anchor.as_raw_fd(), times.as_ptr()) } != 0 {
            return Err(io::Error::last_os_error())
                .context("cannot restore publication timestamps");
        }
        Ok(())
    }
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
        let flags = if self.directory {
            libc::AT_REMOVEDIR
        } else {
            0
        };
        if unsafe { libc::unlinkat(self.parent.as_raw_fd(), self.name.as_ptr(), flags) } != 0 {
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
            directory: false,
            _anchor: Arc::clone(&file),
        };
        return Ok((file, temporary));
    }
    anyhow::bail!("could not allocate a private publication filename")
}
