use port_rs::install::publication::Root;
use std::{
    fs,
    io::{self, Read},
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};
#[test]
fn publishes_complete_file_and_supports_rollback_or_persistence() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("bin")).unwrap();
    let root = Root::open(temp.path()).unwrap();
    let receipt = root
        .publish_file(Path::new("bin/app"), &mut &b"payload"[..], 0o755)
        .unwrap();
    assert_eq!(fs::read(temp.path().join("bin/app")).unwrap(), b"payload");
    assert_eq!(
        fs::metadata(temp.path().join("bin/app"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o755
    );
    assert_eq!(fs::read_dir(temp.path().join("bin")).unwrap().count(), 1);
    drop(receipt);
    assert_eq!(fs::read_dir(temp.path().join("bin")).unwrap().count(), 0);
    root.publish_file(Path::new("bin/app"), &mut &b"retained"[..], 0o640)
        .unwrap()
        .persist();
    assert_eq!(fs::read(temp.path().join("bin/app")).unwrap(), b"retained");
}
#[test]
fn existing_files_and_dangling_links_are_never_overwritten() {
    let temp = tempfile::tempdir().unwrap();
    let root = Root::open(temp.path()).unwrap();
    fs::write(temp.path().join("app"), b"original").unwrap();
    symlink("missing", temp.path().join("link")).unwrap();
    for name in ["app", "link"] {
        assert!(
            root.publish_file(Path::new(name), &mut &b"replacement"[..], 0o644)
                .is_err()
        );
    }
    assert_eq!(fs::read(temp.path().join("app")).unwrap(), b"original");
    assert_eq!(
        fs::read_link(temp.path().join("link")).unwrap(),
        Path::new("missing")
    );
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
}
#[test]
fn refuses_path_escape_and_symlink_parents() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = Root::open(temp.path()).unwrap();
    symlink(outside.path(), temp.path().join("linked")).unwrap();
    for name in ["../escape", "/absolute", "linked/app", "missing/app", ""] {
        assert!(
            root.publish_file(Path::new(name), &mut &b"x"[..], 0o644)
                .is_err()
        );
    }
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    assert!(Root::open(&temp.path().join("linked/")).is_err());
}
struct Broken(bool);
impl Read for Broken {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.0 {
            return Err(io::Error::other("deliberate read failure"));
        }
        self.0 = true;
        buffer[0] = b'x';
        Ok(1)
    }
}
#[test]
fn failed_stream_leaves_neither_partial_file_nor_temporary_file() {
    let temp = tempfile::tempdir().unwrap();
    let root = Root::open(temp.path()).unwrap();
    assert!(
        root.publish_file(Path::new("app"), &mut Broken(false), 0o644)
            .is_err()
    );
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}
#[test]
fn rollback_preserves_a_replacement_inode() {
    let temp = tempfile::tempdir().unwrap();
    let root = Root::open(temp.path()).unwrap();
    let receipt = root
        .publish_file(Path::new("app"), &mut &b"original"[..], 0o644)
        .unwrap();
    fs::rename(temp.path().join("app"), temp.path().join("moved")).unwrap();
    fs::write(temp.path().join("app"), b"replacement").unwrap();
    assert!(receipt.rollback().is_err());
    assert_eq!(fs::read(temp.path().join("app")).unwrap(), b"replacement");
    assert_eq!(fs::read(temp.path().join("moved")).unwrap(), b"original");
}

#[test]
fn rollback_uses_the_held_parent_even_if_its_old_path_becomes_a_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let selected = temp.path().join("selected");
    fs::create_dir(&selected).unwrap();
    let root = Root::open(&selected).unwrap();
    let receipt = root
        .publish_file(Path::new("app"), &mut &b"ours"[..], 0o644)
        .unwrap();
    let moved = temp.path().join("moved");
    fs::rename(&selected, &moved).unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("app"), b"preserve").unwrap();
    symlink(outside.path(), &selected).unwrap();
    receipt.rollback().unwrap();
    assert!(!moved.join("app").exists());
    assert_eq!(fs::read(outside.path().join("app")).unwrap(), b"preserve");
}

#[test]
fn hardlinks_share_inode_and_survive_rollback_of_the_original_name() {
    use std::os::unix::fs::MetadataExt;
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("bin")).unwrap();
    let root = Root::open(temp.path()).unwrap();
    let original = root
        .publish_file(Path::new("app"), &mut &b"payload"[..], 0o755)
        .unwrap();
    let alias = root
        .publish_hardlink(Path::new("bin/alias"), &original)
        .unwrap();
    assert_eq!(
        fs::metadata(temp.path().join("app")).unwrap().ino(),
        fs::metadata(temp.path().join("bin/alias")).unwrap().ino()
    );
    original.rollback().unwrap();
    assert!(!temp.path().join("app").exists());
    assert_eq!(fs::read(temp.path().join("bin/alias")).unwrap(), b"payload");
    assert_eq!(
        fs::metadata(temp.path().join("bin/alias"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    alias.persist();
    assert_eq!(fs::read(temp.path().join("bin/alias")).unwrap(), b"payload");
}

#[test]
fn hardlink_rollback_removes_only_its_own_name_and_never_overwrites() {
    let temp = tempfile::tempdir().unwrap();
    let root = Root::open(temp.path()).unwrap();
    let original = root
        .publish_file(Path::new("app"), &mut &b"payload"[..], 0o644)
        .unwrap();
    fs::write(temp.path().join("occupied"), b"keep").unwrap();
    symlink("missing", temp.path().join("dangling")).unwrap();
    for name in ["app", "occupied", "dangling"] {
        assert!(root.publish_hardlink(Path::new(name), &original).is_err());
    }
    let alias = root
        .publish_hardlink(Path::new("alias"), &original)
        .unwrap();
    drop(alias);
    assert!(!temp.path().join("alias").exists());
    assert_eq!(fs::read(temp.path().join("app")).unwrap(), b"payload");
    assert_eq!(fs::read(temp.path().join("occupied")).unwrap(), b"keep");
    assert_eq!(
        fs::read_link(temp.path().join("dangling")).unwrap(),
        Path::new("missing")
    );
}

#[test]
fn hardlink_source_replacement_and_destination_parent_links_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = Root::open(temp.path()).unwrap();
    let original = root
        .publish_file(Path::new("app"), &mut &b"payload"[..], 0o644)
        .unwrap();
    symlink(outside.path(), temp.path().join("parent")).unwrap();
    assert!(
        root.publish_hardlink(Path::new("parent/alias"), &original)
            .is_err()
    );
    fs::rename(temp.path().join("app"), temp.path().join("old")).unwrap();
    symlink("old", temp.path().join("app")).unwrap();
    assert!(
        root.publish_hardlink(Path::new("alias"), &original)
            .is_err()
    );
    assert!(!temp.path().join("alias").exists());
    assert!(original.rollback().is_err());
    assert_eq!(fs::read(temp.path().join("old")).unwrap(), b"payload");
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
fn creates_directories_with_exact_modes_and_shares_existing_directories() {
    let temp = tempfile::tempdir().unwrap();
    let root = Root::open(temp.path()).unwrap();
    let directory = root
        .create_directory(Path::new("bin"), 0o751)
        .unwrap()
        .unwrap();
    assert_eq!(
        fs::metadata(temp.path().join("bin"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o751
    );
    assert!(
        root.create_directory(Path::new("bin"), 0o700)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        fs::metadata(temp.path().join("bin"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o751
    );
    directory.persist();
    assert!(temp.path().join("bin").is_dir());
}

#[test]
fn nested_publication_can_roll_back_files_then_directories() {
    let temp = tempfile::tempdir().unwrap();
    let root = Root::open(temp.path()).unwrap();
    let parent = root
        .create_directory(Path::new("lib"), 0o755)
        .unwrap()
        .unwrap();
    let child = root
        .create_directory(Path::new("lib/app"), 0o755)
        .unwrap()
        .unwrap();
    let file = root
        .publish_file(Path::new("lib/app/data"), &mut &b"payload"[..], 0o644)
        .unwrap();
    file.rollback().unwrap();
    child.rollback().unwrap();
    parent.rollback().unwrap();
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}

#[test]
fn directory_rollback_preserves_nonempty_and_replaced_directories() {
    let temp = tempfile::tempdir().unwrap();
    let root = Root::open(temp.path()).unwrap();
    let directory = root
        .create_directory(Path::new("nonempty"), 0o755)
        .unwrap()
        .unwrap();
    fs::write(temp.path().join("nonempty/user-file"), b"preserve").unwrap();
    assert!(directory.rollback().is_err());
    assert_eq!(
        fs::read(temp.path().join("nonempty/user-file")).unwrap(),
        b"preserve"
    );
    let directory = root
        .create_directory(Path::new("original"), 0o755)
        .unwrap()
        .unwrap();
    fs::rename(temp.path().join("original"), temp.path().join("moved")).unwrap();
    fs::create_dir(temp.path().join("original")).unwrap();
    assert!(directory.rollback().is_err());
    assert!(temp.path().join("original").is_dir());
    assert!(temp.path().join("moved").is_dir());
}

#[test]
fn directory_creation_rejects_symlinks_files_and_missing_parents() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = Root::open(temp.path()).unwrap();
    symlink(outside.path(), temp.path().join("link")).unwrap();
    fs::write(temp.path().join("file"), b"keep").unwrap();
    for path in ["link", "link/child", "file", "missing/child", "../escape"] {
        assert!(root.create_directory(Path::new(path), 0o755).is_err());
    }
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    assert_eq!(fs::read(temp.path().join("file")).unwrap(), b"keep");
    let directory = root
        .create_directory(Path::new("empty"), 0o755)
        .unwrap()
        .unwrap();
    drop(directory);
    assert!(!temp.path().join("empty").exists());
}

#[test]
#[cfg(target_os = "macos")]
fn symlink_publication_preserves_absolute_relative_and_dangling_targets() {
    let temp = tempfile::tempdir().unwrap();
    let root = Root::open(temp.path()).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("target");
    fs::write(&target, b"preserve").unwrap();
    for (name, target) in [
        ("absolute", target.as_path()),
        ("relative", Path::new("../relative target")),
        ("dangling", Path::new("missing")),
    ] {
        let receipt = root.publish_symlink(Path::new(name), target).unwrap();
        assert_eq!(fs::read_link(temp.path().join(name)).unwrap(), target);
        receipt.rollback().unwrap();
        assert!(fs::symlink_metadata(temp.path().join(name)).is_err());
    }
    assert_eq!(fs::read(target).unwrap(), b"preserve");
    root.publish_symlink(Path::new("kept"), Path::new("missing"))
        .unwrap()
        .persist();
    assert_eq!(
        fs::read_link(temp.path().join("kept")).unwrap(),
        Path::new("missing")
    );
}

#[test]
#[cfg(target_os = "macos")]
fn symlink_collisions_and_linked_parents_never_overwrite_or_escape() {
    let temp = tempfile::tempdir().unwrap();
    let root = Root::open(temp.path()).unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("file"), b"keep").unwrap();
    fs::create_dir(temp.path().join("directory")).unwrap();
    symlink("missing", temp.path().join("link")).unwrap();
    symlink(outside.path(), temp.path().join("parent")).unwrap();
    for path in ["file", "directory", "link", "parent/escape", "../escape"] {
        assert!(
            root.publish_symlink(Path::new(path), Path::new("target"))
                .is_err()
        );
    }
    assert_eq!(fs::read(temp.path().join("file")).unwrap(), b"keep");
    assert_eq!(
        fs::read_link(temp.path().join("link")).unwrap(),
        Path::new("missing")
    );
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
#[cfg(target_os = "macos")]
fn symlink_receipt_rejects_hardlinking_and_preserves_replacement_nodes() {
    let temp = tempfile::tempdir().unwrap();
    let root = Root::open(temp.path()).unwrap();
    let receipt = root
        .publish_symlink(Path::new("link"), Path::new("missing"))
        .unwrap();
    assert!(root.publish_hardlink(Path::new("alias"), &receipt).is_err());
    fs::rename(temp.path().join("link"), temp.path().join("old-link")).unwrap();
    fs::write(temp.path().join("link"), b"replacement").unwrap();
    assert!(receipt.rollback().is_err());
    assert_eq!(fs::read(temp.path().join("link")).unwrap(), b"replacement");
    assert_eq!(
        fs::read_link(temp.path().join("old-link")).unwrap(),
        Path::new("missing")
    );
    let receipt = root
        .publish_symlink(Path::new("temporary"), Path::new("missing"))
        .unwrap();
    drop(receipt);
    assert!(fs::symlink_metadata(temp.path().join("temporary")).is_err());
}
