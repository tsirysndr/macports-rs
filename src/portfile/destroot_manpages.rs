//! In-process man-page normalization for destroot finish.
use super::*;
use flate2::{Compression, GzBuilder, read::MultiGzDecoder};
use std::{
    fs::{self, FileTimes, OpenOptions},
    io::{self, Read},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt, symlink},
        io::AsRawFd,
    },
    path::{Path, PathBuf},
};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ManpageCompression {
    pub compressed: usize,
    pub relinked: usize,
}
impl Session {
    /// Compress/recompress staged man pages and repair their symlinks. The host
    /// still owns the remaining destroot lifecycle and target-state recording.
    pub fn compress_destroot_manpages(&mut self) -> Result<ManpageCompression> {
        check_supported(&mut self.interp, self.id)?;
        ensure!(
            self.interp
                .context::<State>(self.id)
                .system_executor
                .is_some(),
            "man-page compression requires an execution-enabled session"
        );
        let root = PathBuf::from(self.scalar("destroot")?);
        let prefix = self.scalar("prefix")?;
        let man = PathBuf::from(format!("{}{prefix}/share/man", root.display()));
        let mut result = ManpageCompression::default();
        if !physical_directory(&man) {
            return Ok(result);
        }
        ensure!(
            fs::canonicalize(&man)?.starts_with(fs::canonicalize(&root)?),
            "man-page directory resolves outside destroot: {}",
            man.display()
        );
        let mut links = Vec::new();
        for directory in entries(&man)? {
            let name = directory
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            let Some(section) = name
                .strip_prefix("man")
                .or_else(|| name.strip_prefix("cat"))
            else {
                continue;
            };
            if section.chars().count() != 1 || !physical_directory(&directory) {
                continue;
            }
            for path in entries(&directory)? {
                let meta = fs::symlink_metadata(&path)?;
                if meta.file_type().is_symlink() {
                    links.push(path);
                    continue;
                }
                if !meta.is_file() {
                    continue;
                }
                let filename = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| anyhow::anyhow!("man-page filename is not valid UTF-8"))?;
                let (base, format) = if let Some(base) = filename.strip_suffix(".gz") {
                    (base, Format::Gzip)
                } else if let Some(base) = filename.strip_suffix(".bz2") {
                    (base, Format::Bzip2)
                } else {
                    (filename, Format::Plain)
                };
                if !page_name(base, section) {
                    continue;
                }
                let target = directory.join(format!("{base}.gz"));
                compress(&path, &target, format)?;
                result.compressed += 1;
            }
        }
        if result.compressed != 0 {
            // Retry chains after their targets have been repaired. Cycles and
            // missing targets remain unchanged; each successful pass shrinks.
            loop {
                let before = links.len();
                let mut remaining = Vec::new();
                for path in links {
                    if relink(&root, &path)? {
                        result.relinked += 1;
                    } else {
                        remaining.push(path);
                    }
                }
                if remaining.len() == before {
                    break;
                }
                links = remaining;
            }
        }
        Ok(result)
    }
}
fn physical_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
}
fn entries(path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut entries = fs::read_dir(path)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<io::Result<Vec<_>>>()?;
    entries.sort();
    Ok(entries)
}
fn page_name(name: &str, section: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, suffix)| {
        suffix
            .strip_prefix(section)
            .is_some_and(|rest| rest.bytes().all(|byte| byte.is_ascii_lowercase()))
    })
}
enum Format {
    Plain,
    Gzip,
    Bzip2,
}
fn compress(source: &Path, target: &Path, format: Format) -> Result<()> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(source)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file(),
        "man page is not a regular file: {}",
        source.display()
    );
    let mut reader: Box<dyn Read> = match format {
        Format::Plain => Box::new(file),
        Format::Gzip => Box::new(MultiGzDecoder::new(file)),
        Format::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(file)),
    };
    let mut temporary = tempfile::NamedTempFile::new_in(target.parent().unwrap())?;
    let mut encoder = GzBuilder::new()
        .mtime(0)
        .operating_system(3)
        .write(&mut temporary, Compression::best());
    io::copy(&mut reader, &mut encoder)?;
    encoder.finish()?;
    let current = temporary.as_file().metadata()?;
    if (current.uid(), current.gid()) != (metadata.uid(), metadata.gid()) {
        // SAFETY: temporary owns this live descriptor; IDs came from stat.
        if unsafe {
            libc::fchown(
                temporary.as_file().as_raw_fd(),
                metadata.uid(),
                metadata.gid(),
            )
        } != 0
        {
            return Err(io::Error::last_os_error().into());
        }
    }
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o444))?;
    temporary.as_file().set_times(
        FileTimes::new()
            .set_accessed(metadata.accessed()?)
            .set_modified(metadata.modified()?),
    )?;
    temporary.persist(target).map_err(|error| error.error)?;
    if source != target {
        fs::remove_file(source)?;
    }
    Ok(())
}
fn gzip_suffix(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".gz");
    PathBuf::from(name)
}
fn relink(root: &Path, path: &Path) -> Result<bool> {
    let target = fs::read_link(path)?;
    if target
        .extension()
        .is_some_and(|extension| extension == "gz")
    {
        return Ok(false);
    }
    let check = if target.is_absolute() {
        root.join(target.strip_prefix("/")?)
    } else {
        path.parent().unwrap().join(&target)
    };
    if !gzip_suffix(&check).is_file() {
        return Ok(false);
    }
    let new_target = gzip_suffix(&target);
    if path.extension().is_some_and(|extension| extension == "gz") {
        let temporary = tempfile::tempdir_in(path.parent().unwrap())?;
        let link = temporary.path().join("link");
        symlink(new_target, &link)?;
        fs::rename(link, path)?;
    } else {
        let new_path = gzip_suffix(path);
        // Create without overwriting a colliding page or link.
        symlink(new_target, &new_path)?;
        if let Err(error) = fs::remove_file(path) {
            let _ = fs::remove_file(new_path);
            return Err(error.into());
        }
    }
    Ok(true)
}
