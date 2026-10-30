use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_compact::{KeyPair, Seed};
use port_rs::install::signature::{TrustedKey, verify};
use sha2::{Digest, Sha256};
use std::io::{self, Cursor, Read};

fn envelope(packet: &[u8]) -> String {
    format!("untrusted comment: fixture\n{}\n", STANDARD.encode(packet))
}
fn key(seed: u8, id: [u8; 8]) -> (KeyPair, TrustedKey) {
    let pair = KeyPair::from_seed(Seed::new([seed; 32]));
    let mut packet = b"Ed".to_vec();
    packet.extend(id);
    packet.extend(pair.pk.as_ref());
    let public = TrustedKey::parse(&envelope(&packet)).unwrap();
    (pair, public)
}
fn signed(pair: &KeyPair, id: [u8; 8], message: &[u8]) -> String {
    let mut packet = b"Ed".to_vec();
    packet.extend(id);
    packet.extend(pair.sk.sign(message, None).as_ref());
    envelope(&packet)
}
fn hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn independent_rfc8032_vector_verifies_in_signify_envelope() {
    // RFC 8032 section 7.1, TEST 1. No signing code is involved in this vector.
    let id = [7; 8];
    let mut public = b"Ed".to_vec();
    public.extend(id);
    public.extend(hex(
        "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
    ));
    let mut signature = b"Ed".to_vec();
    signature.extend(id);
    signature.extend(hex(concat!(
        "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155",
        "5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
    )));
    let key = TrustedKey::parse(&envelope(&public)).unwrap();
    let result = verify(Cursor::new([]), &envelope(&signature), &[key]).unwrap();
    assert_eq!(result.bytes, 0);
    assert_eq!(result.key_id, id);
    assert_eq!(result.sha256.as_slice(), Sha256::digest([]).as_slice());
}
#[test]
fn streams_large_archives_and_returns_digest_of_authenticated_bytes() {
    let bytes = vec![42u8; 2 * 1024 * 1024 + 7];
    let (pair, public) = key(1, [2; 8]);
    struct Chunked(Cursor<Vec<u8>>);
    impl Read for Chunked {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            assert!(buffer.len() <= 64 * 1024);
            let len = buffer.len().min(113);
            self.0.read(&mut buffer[..len])
        }
    }
    let signature = signed(&pair, [2; 8], &bytes);
    let result = verify(Chunked(Cursor::new(bytes.clone())), &signature, &[public]).unwrap();
    assert_eq!(result.bytes, bytes.len() as u64);
    assert_eq!(result.sha256.as_slice(), Sha256::digest(&bytes).as_slice());
}
#[test]
fn tampering_truncation_and_appended_bytes_fail() {
    let (pair, public) = key(1, [2; 8]);
    let signature = signed(&pair, [2; 8], b"archive payload");
    for message in [
        b"archive payloa".as_slice(),
        b"archive payload!",
        b"Archive payload",
    ] {
        assert!(
            verify(
                Cursor::new(message),
                &signature,
                std::slice::from_ref(&public)
            )
            .is_err()
        );
    }
}
#[test]
fn key_id_matching_never_substitutes_for_signature_authentication() {
    let (pair, public) = key(1, [2; 8]);
    let (_, wrong_same_id) = key(3, [2; 8]);
    let (_, wrong_id) = key(1, [9; 8]);
    let signature = signed(&pair, [2; 8], b"archive");
    assert!(verify(Cursor::new(b"archive"), &signature, &[wrong_id]).is_err());
    assert!(
        verify(
            Cursor::new(b"archive"),
            &signature,
            std::slice::from_ref(&wrong_same_id)
        )
        .is_err()
    );
    assert!(
        verify(
            Cursor::new(b"archive"),
            &signature,
            &[wrong_same_id, public]
        )
        .is_ok()
    );
}
#[test]
fn malformed_envelopes_and_algorithms_are_rejected() {
    let (pair, public) = key(1, [2; 8]);
    let signature = signed(&pair, [2; 8], b"archive");
    for text in [
        "",
        "comment\nAAAA",
        "untrusted comment: x\n!",
        "untrusted comment: x\n",
        &format!("{signature}extra\n"),
        &"x".repeat(4097),
    ] {
        assert!(verify(Cursor::new(b"archive"), text, std::slice::from_ref(&public)).is_err());
        assert!(TrustedKey::parse(text).is_err());
    }
    for length in [0, 2, 42, 73, 75] {
        assert!(
            verify(
                Cursor::new(b"archive"),
                &envelope(&vec![0; length]),
                std::slice::from_ref(&public)
            )
            .is_err()
        );
    }
    let mut packet = vec![0u8; 74];
    packet[..2].copy_from_slice(b"ED");
    assert!(
        verify(Cursor::new(b"archive"), &envelope(&packet), &[public])
            .unwrap_err()
            .to_string()
            .contains("unsupported signature algorithm")
    );
}
#[test]
fn interrupted_reads_retry_and_other_io_errors_abort() {
    let (pair, public) = key(1, [2; 8]);
    let signature = signed(&pair, [2; 8], b"archive");
    struct Interrupted {
        once: bool,
        source: Cursor<&'static [u8]>,
    }
    impl Read for Interrupted {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            if !self.once {
                self.once = true;
                return Err(io::ErrorKind::Interrupted.into());
            }
            self.source.read(bytes)
        }
    }
    assert!(
        verify(
            Interrupted {
                once: false,
                source: Cursor::new(b"archive")
            },
            &signature,
            std::slice::from_ref(&public)
        )
        .is_ok()
    );
    struct Failed;
    impl Read for Failed {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::PermissionDenied.into())
        }
    }
    assert!(
        verify(Failed, &signature, &[public])
            .unwrap_err()
            .to_string()
            .contains("cannot read archive")
    );
}
#[test]
fn invalid_trust_sets_fail_before_reading_archive() {
    struct Unreadable;
    impl Read for Unreadable {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("must not read")
        }
    }
    let (pair, public) = key(1, [2; 8]);
    let signature = signed(&pair, [2; 8], b"archive");
    assert!(verify(Unreadable, &signature, &[]).is_err());
    assert!(verify(Unreadable, &signature, &vec![public; 65]).is_err());
}
#[test]
fn public_key_comments_are_untrusted_and_crlf_is_accepted() {
    let (pair, public) = key(1, [2; 8]);
    let signature = signed(&pair, [2; 8], b"archive")
        .replace("fixture", "arbitrary name")
        .replace('\n', "\r\n");
    assert!(verify(Cursor::new(b"archive"), &signature, &[public]).is_ok());
    assert_eq!(
        TrustedKey::macports_archive_key().unwrap().id(),
        [0x7e, 0x3e, 0x18, 0xab, 0x04, 0x27, 0x25, 0x42]
    );
}

#[test]
fn staging_preserves_verified_bytes_when_original_changes() {
    use port_rs::install::archive::VerifiedArchive;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input");
    std::fs::write(&path, b"archive").unwrap();
    let (pair, public) = key(1, [2; 8]);
    let signature = signed(&pair, [2; 8], b"archive");
    let mut staged = VerifiedArchive::stage(
        std::fs::File::open(&path).unwrap(),
        &signature,
        &[public],
        directory.path(),
        7,
    )
    .unwrap();
    std::fs::write(&path, b"replaced").unwrap();
    for _ in 0..2 {
        let mut data = Vec::new();
        staged.reader().unwrap().read_to_end(&mut data).unwrap();
        assert_eq!(data, b"archive");
    }
    assert_eq!(staged.verification().bytes, 7);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}
#[test]
fn staging_rejects_oversized_or_tampered_sources_without_publishing_files() {
    use port_rs::install::archive::VerifiedArchive;
    let directory = tempfile::tempdir().unwrap();
    let (pair, public) = key(1, [2; 8]);
    let signature = signed(&pair, [2; 8], b"archive");
    for (message, limit) in [
        (b"archive".as_slice(), 6),
        (b"archive!", 7),
        (b"changed", 100),
        (b"archive", 0),
    ] {
        assert!(
            VerifiedArchive::stage(
                Cursor::new(message),
                &signature,
                std::slice::from_ref(&public),
                directory.path(),
                limit
            )
            .is_err()
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
