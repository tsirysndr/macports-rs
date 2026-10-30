//! Filesystem preflight and reversible publication of verified, prepared images.
mod publish;
use super::{
    extract::PreparedImage,
    inspect::{Kind, Member},
};
use anyhow::{Context, Result, ensure};
pub use publish::PublishedImage;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, PartialEq, Eq)]
pub enum ConflictKind {
    /// A payload file would replace an existing object, including a dangling link.
    ExistingFile,
    /// A required directory is a file, symlink, or other non-directory object.
    NonDirectory,
}
#[derive(Debug)]
pub struct Conflict {
    pub path: PathBuf,
    pub kind: ConflictKind,
}
#[derive(Debug)]
pub struct Directory {
    pub destination: PathBuf,
    pub exists: bool,
    /// Explicit archive directory metadata; implicit parents have no member.
    pub member_path: Option<String>,
}
#[derive(Debug)]
pub struct File<'a> {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub installed_path: String,
    pub member: &'a Member,
}
/// This is a snapshot, not an authorization to mutate the prefix. A coordinator
/// must hold the registry lock, check ownership, and revalidate paths while
/// publishing. Keeping the image borrowed retains its private extraction tree.
pub struct Plan<'a> {
    pub image: &'a PreparedImage,
    pub root: PathBuf,
    pub directories: Vec<Directory>,
    pub files: Vec<File<'a>>,
    pub conflicts: Vec<Conflict>,
}
impl<'a> Plan<'a> {
    /// Map archive paths beneath an existing physical root (normally `/`).
    /// Existing physical directories may be shared; payload files are never
    /// silently overwritten. Ancestor symlinks are reported, never followed.
    pub fn inspect(image: &'a PreparedImage, root: &Path) -> Result<Self> {
        // Strip trailing separators before lstat; otherwise a symlink followed
        // by '/' is resolved as a directory by the kernel.
        let root: PathBuf = root.components().collect();
        ensure!(
            fs::symlink_metadata(&root)?.is_dir(),
            "activation root must be a physical directory"
        );
        let root = root.canonicalize()?;
        let mut directory_names: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();
        let mut files = Vec::new();
        let payload = image.payload();
        for (name, member) in &image.inspection().members {
            if name.is_empty() || name.starts_with('+') {
                continue;
            }
            let relative = Path::new(name);
            ensure!(
                relative
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))),
                "invalid activation member path {name:?}"
            );
            let mut parent = relative.parent();
            while let Some(path) = parent.filter(|p| !p.as_os_str().is_empty()) {
                directory_names.entry(path.to_owned()).or_default();
                parent = path.parent();
            }
            if member.kind == Kind::Directory {
                directory_names.insert(relative.to_owned(), Some(name.clone()));
            } else {
                files.push(File {
                    source: payload.join(relative),
                    destination: root.join(relative),
                    installed_path: format!("/{name}"),
                    member,
                });
            }
        }
        let mut directory_names: Vec<_> = directory_names.into_iter().collect();
        directory_names.sort_by(|a, b| {
            a.0.components()
                .count()
                .cmp(&b.0.components().count())
                .then_with(|| a.0.cmp(&b.0))
        });
        let mut conflicts = Vec::new();
        let mut blocked: BTreeSet<PathBuf> = BTreeSet::new();
        let mut directories = Vec::new();
        for (relative, member_path) in directory_names {
            let destination = root.join(relative);
            if destination
                .ancestors()
                .any(|parent| blocked.contains(parent))
            {
                continue;
            }
            let exists = match metadata(&destination)? {
                Some(metadata) if metadata.is_dir() => true,
                Some(_) => {
                    blocked.insert(destination.clone());
                    conflicts.push(Conflict {
                        path: destination,
                        kind: ConflictKind::NonDirectory,
                    });
                    continue;
                }
                None => false,
            };
            directories.push(Directory {
                destination,
                exists,
                member_path,
            });
        }
        for file in &files {
            if file
                .destination
                .ancestors()
                .any(|parent| blocked.contains(parent))
            {
                continue;
            }
            if metadata(&file.destination)?.is_some() {
                conflicts.push(Conflict {
                    path: file.destination.clone(),
                    kind: ConflictKind::ExistingFile,
                });
            }
        }
        Ok(Self {
            image,
            root,
            directories,
            files,
            conflicts,
        })
    }
    /// Build and check the complete registry mapping while the caller retains
    /// the writer transaction. This does not activate the image or publish files.
    pub fn check_registry(
        &self,
        transaction: &crate::registry::write::WriteTransaction<'_>,
        image_id: i64,
    ) -> Result<Vec<crate::registry::write::ActivationFile>> {
        self.ensure_clear()?;
        let identity = transaction.image_identity(image_id)?;
        let contents = &self.image.inspection().contents;
        let variants: String = contents
            .variants
            .iter()
            .map(|variant| format!("+{variant}"))
            .collect();
        let mut archs: Vec<_> = identity.archs.split_whitespace().collect();
        archs.sort_unstable();
        let mut expected_archs: Vec<_> = contents.archs.iter().map(String::as_str).collect();
        expected_archs.sort_unstable();
        ensure!(
            identity.name.eq_ignore_ascii_case(&contents.name)
                && identity.version == contents.version
                && u64::try_from(identity.revision).ok() == Some(contents.revision)
                && u64::try_from(identity.epoch).ok() == Some(contents.epoch)
                && identity.variants == variants
                && archs == expected_archs,
            "registered image identity does not match the verified archive"
        );
        let mappings = self
            .files
            .iter()
            .map(|file| {
                Ok(crate::registry::write::ActivationFile {
                    path: file.installed_path.clone(),
                    actual_path: file
                        .destination
                        .to_str()
                        .context("activation destination is not UTF-8")?
                        .to_owned(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        transaction.check_activation(image_id, &mappings)?;
        Ok(mappings)
    }
    /// Refuse publication until all conflicts are explicitly resolved.
    pub fn ensure_clear(&self) -> Result<()> {
        ensure!(
            self.conflicts.is_empty(),
            "activation conflicts:\n{}",
            self.conflicts
                .iter()
                .map(|c| format!("{}: {:?}", c.path.display(), c.kind))
                .collect::<Vec<_>>()
                .join("\n")
        );
        Ok(())
    }
}
fn metadata(path: &Path) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("cannot inspect activation path {}", path.display()))
        }
    }
}
