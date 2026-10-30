use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_compact::{KeyPair, Seed};
use port_rs::install::{
    pubkeys,
    signature::{self, TrustedKey},
};

fn envelope(bytes: &[u8]) -> String {
    let mut packet = b"Ed12345678".to_vec();
    packet.extend(bytes);
    format!("untrusted comment: fixture\n{}\n", STANDARD.encode(packet))
}
#[test]
fn configured_keys_authenticate_bytes_and_bad_entries_do_not_hide_valid_keys() {
    let root = tempfile::tempdir().unwrap();
    let key = KeyPair::from_seed(Seed::new([42; 32]));
    std::fs::create_dir(root.path().join("etc")).unwrap();
    std::fs::write(root.path().join("valid.pub"), envelope(key.pk.as_ref())).unwrap();
    std::fs::write(root.path().join("bad.pub"), "not a key").unwrap();
    let config = root.path().join("etc/pubkeys.conf");
    std::fs::write(
        &config,
        "# comment\n missing.pub\nbad.pub\nvalid.pub\nvalid.pub\nlegacy.pem\n",
    )
    .unwrap();
    let trust = pubkeys::load_signify(&config, root.path(), vec![]).unwrap();
    assert_eq!(trust.keys.len(), 1);
    assert_eq!(trust.warnings.len(), 2);
    let bytes = b"archive data";
    let signature = envelope(key.sk.sign(bytes, None).as_ref());
    assert_eq!(
        signature::verify(bytes.as_slice(), &signature, &trust.keys)
            .unwrap()
            .bytes,
        bytes.len() as u64
    );
    assert!(signature::verify(b"tampered".as_slice(), &signature, &trust.keys).is_err());
}
#[test]
fn missing_configuration_preserves_site_keys_and_bounded_invalid_files_warn() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("pubkeys.conf");
    let trust = pubkeys::load_signify(
        &config,
        root.path(),
        vec![TrustedKey::macports_archive_key().unwrap()],
    )
    .unwrap();
    assert_eq!(trust.keys.len(), 1);
    assert!(trust.warnings.is_empty());
    std::fs::write(root.path().join("large.pub"), vec![b'x'; 4097]).unwrap();
    std::fs::write(&config, "large.pub\n").unwrap();
    let trust = pubkeys::load_signify(&config, root.path(), vec![]).unwrap();
    assert!(trust.keys.is_empty());
    assert_eq!(trust.warnings.len(), 1);
    std::fs::write(&config, vec![b'x'; 1024 * 1024 + 1]).unwrap();
    assert!(pubkeys::load_signify(&config, root.path(), vec![]).is_err());
}
#[test]
fn configuration_path_override_selects_the_trust_file() {
    use port_rs::config::{Config, Paths};
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("key.pub"),
        include_str!("../src/install/keys/macports-archives-2025.pub"),
    )
    .unwrap();
    std::fs::write(root.path().join("custom.conf"), "key.pub\n").unwrap();
    let mut config = Config::default();
    config
        .apply("pubkeys_conf custom.conf", |path| {
            Ok(root.path().join(path).to_str().unwrap().into())
        })
        .unwrap();
    let paths = Paths {
        configuration_directory: root.path().join("unused"),
        working_directory: root.path().to_owned(),
        user_home: None,
        portsrc: None,
    };
    assert_eq!(paths.archive_keys(&config, vec![]).unwrap().keys.len(), 1);
    assert!(
        paths
            .archive_keys(&Config::default(), vec![])
            .unwrap()
            .keys
            .is_empty()
    );
}
