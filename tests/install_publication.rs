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
