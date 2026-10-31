//! Private staged archives retain the exact bytes that passed authentication.
use super::signature::{self, TrustedKey, Verification};
use anyhow::{Result, ensure};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

pub struct VerifiedArchive {
    file: File,
    verification: Verification,
    signature: String,
}
impl VerifiedArchive {
    /// Copy and authenticate into an anonymous temporary file. A failure drops the
    /// staging file; the original is never renamed, modified, or extracted.
    pub fn stage(
        input: impl Read,
        signature: &str,
        keys: &[TrustedKey],
        directory: &Path,
        max_bytes: u64,
    ) -> Result<Self> {
        ensure!(max_bytes > 0, "archive size limit must be positive");
        let mut file = tempfile::tempfile_in(directory)?;
        let mut tee = StagingReader {
            input,
            output: &mut file,
            remaining: max_bytes,
        };
        let verification = signature::verify(&mut tee, signature, keys)?;
        file.flush()?;
        file.seek(SeekFrom::Start(0))?;
        Ok(Self {
            file,
            verification,
            signature: signature.to_owned(),
        })
    }
    /// Take exclusive ownership of a private download file without copying it.
    pub(crate) fn authenticate(
        mut file: File,
        signature: &str,
        keys: &[TrustedKey],
    ) -> Result<Self> {
        file.seek(SeekFrom::Start(0))?;
        let verification = signature::verify(&mut file, signature, keys)?;
        file.seek(SeekFrom::Start(0))?;
        Ok(Self {
            file,
            verification,
            signature: signature.to_owned(),
        })
    }
    /// A port image built locally from a destroot. It is trusted because this
    /// process produced it, so there is no signature; its digest still binds
    /// later preparation to these exact bytes.
    pub fn local(mut file: File) -> Result<Self> {
        use sha2::{Digest, Sha256};
        file.seek(SeekFrom::Start(0))?;
        let mut hasher = Sha256::new();
        let bytes = io::copy(&mut file, &mut hasher)?;
        file.seek(SeekFrom::Start(0))?;
        Ok(Self {
            file,
            verification: Verification {
                key_id: [0; 8],
                bytes,
                sha256: hasher.finalize().into(),
            },
            signature: String::new(),
        })
    }
    pub fn verification(&self) -> &Verification {
        &self.verification
    }
    /// Retain the authenticated signature envelope for later cache verification.
    pub fn signature(&self) -> &str {
        &self.signature
    }
    /// Rewind and borrow the staged bytes without exposing a writable handle or
    /// reopening an attacker-replaceable source path.
    pub fn reader(&mut self) -> Result<impl Read + '_> {
        self.file.seek(SeekFrom::Start(0))?;
        Ok(&mut self.file)
    }
}
struct StagingReader<'a, R> {
    input: R,
    output: &'a mut File,
    remaining: u64,
}
impl<R: Read> Read for StagingReader<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        // Read at most one byte beyond the limit to distinguish exact-size EOF
        // from a longer message; never authenticate a silently truncated prefix.
        let limit = self.remaining.saturating_add(1).min(buffer.len() as u64) as usize;
        let count = self.input.read(&mut buffer[..limit])?;
        if count as u64 > self.remaining {
            return Err(io::Error::other("archive exceeds configured size limit"));
        }
        self.output.write_all(&buffer[..count])?;
        self.remaining -= count as u64;
        Ok(count)
    }
}
