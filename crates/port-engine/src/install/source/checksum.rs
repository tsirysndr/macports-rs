use anyhow::{Result, bail};
use md5::Md5;
use ripemd::{Digest, Ripemd160};
use sha1::Sha1;
use sha2::Sha256;
use std::{collections::BTreeMap, fmt::Write, io::Read, path::Path, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Md5,
    Sha1,
    Rmd160,
    Sha256,
    Size,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Md5 => "md5",
            Kind::Sha1 => "sha1",
            Kind::Rmd160 => "rmd160",
            Kind::Sha256 => "sha256",
            Kind::Size => "size",
        }
    }
}

impl FromStr for Kind {
    type Err = anyhow::Error;
    fn from_str(name: &str) -> Result<Self> {
        Ok(match name {
            "md5" => Kind::Md5,
            "sha1" => Kind::Sha1,
            "rmd160" => Kind::Rmd160,
            "sha256" => Kind::Sha256,
            "size" => Kind::Size,
            _ => bail!("unknown checksum type {name}"),
        })
    }
}

pub type Table = BTreeMap<String, Vec<(Kind, String)>>;

const DEFAULT_KINDS: [Kind; 3] = [Kind::Rmd160, Kind::Sha256, Kind::Size];

/// Mirror `portchecksum::parse_checksums`, including its lenient handling of
/// filenames without values and of repeated filenames.
pub fn parse(words: &[String], distfiles: &[String]) -> Table {
    let is_kind = |word: Option<&String>| word.is_some_and(|word| word.parse::<Kind>().is_ok());
    let mut words = words.to_vec();
    if let [only] = distfiles
        && words.len().is_multiple_of(2)
        && words.len() / 2 <= 5
        && is_kind(words.first())
    {
        words.insert(0, only.clone());
    }
    let mut table = Table::new();
    let count = words.len();
    let mut index = 0;
    while index < count {
        let filename = words[index].clone();
        let mut values = table.remove(&filename).unwrap_or_default();
        index += 1;
        loop {
            if let Some(kind) = words.get(index).and_then(|word| word.parse().ok()) {
                values.push((kind, words.get(index + 1).cloned().unwrap_or_default()));
                index += 2;
            } else {
                index -= 1;
                break;
            }
            if index == count {
                break;
            }
        }
        table.insert(filename, values);
        index += 1;
    }
    table
}

/// Hash a file once and return the values for the requested kinds, in order.
pub fn calculate(path: &Path, kinds: &[Kind]) -> Result<Vec<String>> {
    let mut file = std::fs::File::open(path)?;
    let (mut md5, mut sha1, mut rmd160, mut sha256) = (
        Md5::new(),
        Sha1::new(),
        Ripemd160::new(),
        <Sha256 as sha2::Digest>::new(),
    );
    let mut size = 0u64;
    let mut buffer = vec![0; 1 << 16];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let chunk = &buffer[..count];
        size += count as u64;
        for kind in kinds {
            match kind {
                Kind::Md5 => md5.update(chunk),
                Kind::Sha1 => sha1.update(chunk),
                Kind::Rmd160 => rmd160.update(chunk),
                Kind::Sha256 => sha2::Digest::update(&mut sha256, chunk),
                Kind::Size => {}
            }
        }
    }
    let (md5, sha1, rmd160) = (
        hex(&md5.finalize()),
        hex(&sha1.finalize()),
        hex(&rmd160.finalize()),
    );
    let sha256 = hex(&sha2::Digest::finalize(sha256));
    Ok(kinds
        .iter()
        .map(|kind| match kind {
            Kind::Md5 => md5.clone(),
            Kind::Sha1 => sha1.clone(),
            Kind::Rmd160 => rmd160.clone(),
            Kind::Sha256 => sha256.clone(),
            Kind::Size => size.to_string(),
        })
        .collect())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    Missing(String),
    Mismatch {
        distfile: String,
        kind: Kind,
        expected: String,
        actual: String,
    },
}

/// Check every distfile like `portchecksum::checksum_main`, collecting all
/// problems instead of stopping at the first.
pub fn verify(directory: &Path, distfiles: &[String], table: &Table) -> Result<Vec<Problem>> {
    let mut problems = Vec::new();
    for distfile in distfiles {
        let path = directory.join(distfile);
        if !path.is_file() {
            bail!("{distfile} does not exist in {}", directory.display());
        }
        let expected = table.get(distfile).filter(|values| !values.is_empty());
        let Some(expected) = expected else {
            problems.push(Problem::Missing(distfile.clone()));
            continue;
        };
        let kinds: Vec<_> = expected.iter().map(|(kind, _)| *kind).collect();
        let actual = calculate(&path, &kinds)?;
        for ((kind, expected), actual) in expected.iter().zip(actual) {
            if *expected != actual {
                problems.push(Problem::Mismatch {
                    distfile: distfile.clone(),
                    kind: *kind,
                    expected: expected.clone(),
                    actual,
                });
            }
        }
    }
    Ok(problems)
}

/// The replacement line upstream suggests: the Portfile's types followed by
/// any missing default types, one `type value` pair per continuation line.
pub fn suggestion(directory: &Path, distfiles: &[String], table: &Table) -> Result<String> {
    let mut sums = Vec::new();
    for distfile in distfiles {
        if distfiles.len() > 1 {
            sums.push(distfile.clone());
        }
        let path = directory.join(distfile);
        if !path.is_file() {
            bail!("{distfile} does not exist in {}", directory.display());
        }
        let mut kinds: Vec<Kind> = table
            .get(distfile)
            .map(|values| values.iter().map(|(kind, _)| *kind).collect())
            .unwrap_or_default();
        for kind in DEFAULT_KINDS {
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
        for (kind, value) in kinds.iter().zip(calculate(&path, &kinds)?) {
            sums.push(format!("{:<8}{value}", kind.name()));
        }
    }
    Ok(format!(
        "{:<20}{}",
        "checksums",
        sums.join(&format!(" \\\n{:<20}", ""))
    ))
}
