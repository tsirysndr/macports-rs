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

#[test]
fn restores_times_on_held_inodes_and_preserves_unspecified_access_time() {
    use port_rs::install::inspect::Timestamp;
    use std::os::unix::fs::MetadataExt;
    let temp = tempfile::tempdir().unwrap();
    let root = Root::open(temp.path()).unwrap();
    let directory = root
        .create_directory(Path::new("bin"), 0o755)
        .unwrap()
        .unwrap();
    let file = root
        .publish_file(Path::new("bin/app"), &mut &b"data"[..], 0o755)
        .unwrap();
    let alias = root.publish_hardlink(Path::new("alias"), &file).unwrap();
    let access = Timestamp {
        seconds: 1_600_000_000,
        nanoseconds: 123_456_789,
    };
    let modified = Timestamp {
        seconds: -1,
        nanoseconds: 750_000_000,
    };
    file.set_times(Some(access), modified).unwrap();
    let metadata = fs::metadata(temp.path().join("alias")).unwrap();
    assert_eq!(
        (metadata.atime(), metadata.atime_nsec()),
        (access.seconds, i64::from(access.nanoseconds))
    );
    assert_eq!(
        (metadata.mtime(), metadata.mtime_nsec()),
        (modified.seconds, i64::from(modified.nanoseconds))
    );
    let later = Timestamp {
        seconds: 1_700_000_000,
        nanoseconds: 987_654_321,
    };
    alias.set_times(None, later).unwrap();
    let metadata = fs::metadata(temp.path().join("bin/app")).unwrap();
    assert_eq!(
        (metadata.atime(), metadata.atime_nsec()),
        (access.seconds, i64::from(access.nanoseconds))
    );
    assert_eq!(
        (metadata.mtime(), metadata.mtime_nsec()),
        (later.seconds, i64::from(later.nanoseconds))
    );
    directory.set_times(Some(access), later).unwrap();
    let metadata = fs::metadata(temp.path().join("bin")).unwrap();
    assert_eq!(
        (metadata.mtime(), metadata.mtime_nsec()),
        (later.seconds, i64::from(later.nanoseconds))
    );
    assert!(
        file.set_times(
            None,
            Timestamp {
                seconds: 0,
                nanoseconds: 1_000_000_000
            }
        )
        .is_err()
    );
    // Replacement paths never redirect metadata writes to the replacement inode.
    fs::rename(temp.path().join("bin/app"), temp.path().join("moved")).unwrap();
    fs::write(temp.path().join("bin/app"), b"replacement").unwrap();
    let replacement = fs::metadata(temp.path().join("bin/app")).unwrap();
    file.set_times(None, modified).unwrap();
    let unchanged = fs::metadata(temp.path().join("bin/app")).unwrap();
    assert_eq!(
        (replacement.mtime(), replacement.mtime_nsec()),
        (unchanged.mtime(), unchanged.mtime_nsec())
    );
    assert_eq!(fs::metadata(temp.path().join("moved")).unwrap().mtime(), -1);
    file.persist();
    alias.persist();
    directory.persist();
}

#[cfg(target_os = "macos")]
#[test]
fn restores_symlink_times_without_touching_the_target() {
    use port_rs::install::inspect::Timestamp;
    use std::os::unix::fs::MetadataExt;
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("target"), b"untouched").unwrap();
    let before = fs::metadata(temp.path().join("target")).unwrap();
    let root = Root::open(temp.path()).unwrap();
    for (name, target) in [("link", "target"), ("dangling", "missing")] {
        let link = root
            .publish_symlink(Path::new(name), Path::new(target))
            .unwrap();
        let time = Timestamp {
            seconds: 1_600_000_000,
            nanoseconds: 123_456_789,
        };
        link.set_times(Some(time), time).unwrap();
        let metadata = fs::symlink_metadata(temp.path().join(name)).unwrap();
        assert_eq!(
            (metadata.mtime(), metadata.mtime_nsec()),
            (time.seconds, i64::from(time.nanoseconds))
        );
        assert_eq!(
            (metadata.atime(), metadata.atime_nsec()),
            (time.seconds, i64::from(time.nanoseconds))
        );
    }
    let after = fs::metadata(temp.path().join("target")).unwrap();
    assert_eq!(
        (before.mtime(), before.mtime_nsec()),
        (after.mtime(), after.mtime_nsec())
    );
    assert_eq!(
        (before.atime(), before.atime_nsec()),
        (after.atime(), after.atime_nsec())
    );
}

#[test]
fn restores_owner_and_mode_after_chown_and_rejects_invalid_values_before_mutation() {
    use std::os::unix::fs::MetadataExt;
    let temp = tempfile::tempdir().unwrap();
    let root = Root::open(temp.path()).unwrap();
    let owner = fs::metadata(temp.path()).unwrap();
    let (uid, gid) = (u64::from(owner.uid()), u64::from(owner.gid()));
    let directory = root
        .create_directory(Path::new("bin"), 0o700)
        .unwrap()
        .unwrap();
    let file = root
        .publish_file(Path::new("bin/app"), &mut &b"data"[..], 0o600)
        .unwrap();
    let alias = root.publish_hardlink(Path::new("alias"), &file).unwrap();
    file.set_owner_mode(uid, gid, 0o4755).unwrap();
    let metadata = fs::metadata(temp.path().join("alias")).unwrap();
    assert_eq!(
        (
            u64::from(metadata.uid()),
            u64::from(metadata.gid()),
            metadata.mode() & 0o7777
        ),
        (uid, gid, 0o4755)
    );
    for (bad_uid, bad_gid, mode) in [
        (u64::MAX, gid, 0o600),
        (uid, u64::MAX, 0o600),
        (u64::from(u32::MAX), gid, 0o600),
        (uid, u64::from(u32::MAX), 0o600),
        (uid, gid, 0o10000),
    ] {
        assert!(file.set_owner_mode(bad_uid, bad_gid, mode).is_err());
        assert_eq!(
            fs::metadata(temp.path().join("alias")).unwrap().mode() & 0o7777,
            0o4755
        );
    }
    directory.set_owner_mode(uid, gid, 0o750).unwrap();
    assert_eq!(
        fs::metadata(temp.path().join("bin")).unwrap().mode() & 0o7777,
        0o750
    );
    fs::rename(temp.path().join("bin/app"), temp.path().join("moved")).unwrap();
    fs::write(temp.path().join("bin/app"), b"replacement").unwrap();
    let before = fs::metadata(temp.path().join("bin/app")).unwrap();
    file.set_owner_mode(uid, gid, 0o640).unwrap();
    let after = fs::metadata(temp.path().join("bin/app")).unwrap();
    assert_eq!(
        (before.uid(), before.gid(), before.mode()),
        (after.uid(), after.gid(), after.mode())
    );
    assert_eq!(
        fs::metadata(temp.path().join("moved")).unwrap().mode() & 0o7777,
        0o640
    );
    file.persist();
    alias.persist();
    directory.persist();
}

#[cfg(target_os = "macos")]
#[test]
fn restores_symlink_owner_and_mode_without_following_the_target() {
    use std::os::unix::fs::MetadataExt;
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("target"), b"untouched").unwrap();
    fs::set_permissions(
        temp.path().join("target"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let before = fs::metadata(temp.path().join("target")).unwrap();
    let root = Root::open(temp.path()).unwrap();
    for (name, target) in [("link", "target"), ("dangling", "missing")] {
        let link = root
            .publish_symlink(Path::new(name), Path::new(target))
            .unwrap();
        link.set_owner_mode(u64::from(before.uid()), u64::from(before.gid()), 0o755)
            .unwrap();
        let metadata = fs::symlink_metadata(temp.path().join(name)).unwrap();
        assert_eq!(
            (metadata.uid(), metadata.gid(), metadata.mode() & 0o7777),
            (before.uid(), before.gid(), 0o755)
        );
    }
    let after = fs::metadata(temp.path().join("target")).unwrap();
    assert_eq!(
        (
            before.uid(),
            before.gid(),
            before.mode(),
            before.ctime(),
            before.ctime_nsec()
        ),
        (
            after.uid(),
            after.gid(),
            after.mode(),
            after.ctime(),
            after.ctime_nsec()
        )
    );
}
