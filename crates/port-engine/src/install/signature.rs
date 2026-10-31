//! Streaming verification of MacPorts Signify/Ed25519 archive signatures.
//! Trust keys must come from the caller or the bundled release key, never a mirror.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_compact::{PublicKey, Signature};
use sha2::{Digest, Sha256};
use std::io::{ErrorKind, Read};

const MAX_ENVELOPE: usize = 4096;
const MAX_KEYS: usize = 64;
const ARCHIVE_KEY: &str = include_str!("keys/macports-archives-2025.pub");
const PORTS_KEY: &str = include_str!("keys/macports-ports-2025.pub");

#[derive(Clone)]
pub struct TrustedKey {
    id: [u8; 8],
    key: PublicKey,
}
impl TrustedKey {
    /// Parse an explicitly trusted Signify .pub file (not a PEM/RSA key).
    pub fn parse(text: &str) -> Result<Self> {
        let packet = envelope(text, 42)?;
        Ok(Self {
            id: packet[2..10]
                .try_into()
                .expect("validated key packet length"),
            key: PublicKey::from_slice(&packet[10..]).context("invalid Ed25519 public key")?,
        })
    }
    pub fn macports_archive_key() -> Result<Self> {
        Self::parse(ARCHIVE_KEY)
    }
    /// The key that signs the official ports tree tarball and PortIndex files.
    pub fn macports_ports_key() -> Result<Self> {
        Self::parse(PORTS_KEY)
    }
    pub fn id(&self) -> [u8; 8] {
        self.id
    }
}

/// Authentication result for exactly the bytes consumed by `verify`.
/// This does not assert archive contents, platform, freshness, or package identity.
#[derive(Debug, PartialEq, Eq)]
pub struct Verification {
    pub key_id: [u8; 8],
    pub bytes: u64,
    pub sha256: [u8; 32],
}

fn envelope(text: &str, expected: usize) -> Result<Vec<u8>> {
    ensure!(
        text.len() <= MAX_ENVELOPE,
        "Signify envelope exceeds {MAX_ENVELOPE} bytes"
    );
    let mut lines = text.lines();
    ensure!(
        lines
            .next()
            .is_some_and(|line| line.starts_with("untrusted comment: ")),
        "invalid Signify comment header"
    );
    let data = lines.next().context("missing Signify payload")?;
    ensure!(
        lines.all(|line| line.is_empty()),
        "unexpected data after Signify payload"
    );
    let packet = STANDARD
        .decode(data)
        .context("invalid Signify base64 payload")?;
    ensure!(
        packet.len() == expected,
        "invalid Signify packet length (expected {expected} bytes)"
    );
    ensure!(
        &packet[..2] == b"Ed",
        "unsupported signature algorithm; only Signify Ed25519 (Ed) is supported"
    );
    Ok(packet)
}

/// Verify an entire stream in bounded memory. Reads through EOF, retries interrupted
/// reads, and never returns a successful result for a partial or unauthenticated file.
/// The caller must keep these bytes stable before any subsequent use or extraction.
pub fn verify(mut input: impl Read, signature: &str, keys: &[TrustedKey]) -> Result<Verification> {
    ensure!(
        !keys.is_empty() && keys.len() <= MAX_KEYS,
        "provide between 1 and {MAX_KEYS} trusted keys"
    );
    let packet = envelope(signature, 74)?;
    let id: [u8; 8] = packet[2..10]
        .try_into()
        .expect("validated signature packet length");
    let signature = Signature::from_slice(&packet[10..]).context("invalid Ed25519 signature")?;
    let mut states: Vec<_> = keys
        .iter()
        .filter(|key| key.id == id)
        .filter_map(|key| key.key.verify_incremental(&signature).ok())
        .collect();
    ensure!(
        !states.is_empty(),
        "archive signature has no usable matching trusted key"
    );
    let mut sha256 = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = match input.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(error).context("cannot read archive for signature verification");
            }
        };
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(count as u64)
            .context("archive length exceeds u64")?;
        for state in &mut states {
            state.absorb(&buffer[..count]);
        }
        sha256.update(&buffer[..count]);
    }
    ensure!(
        states.iter().any(|state| state.verify().is_ok()),
        "archive signature verification failed"
    );
    Ok(Verification {
        key_id: id,
        bytes,
        sha256: sha256.finalize().into(),
    })
}
