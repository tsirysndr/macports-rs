//! Bounded, seek-based architecture inspection of Mach-O and Unix ar containers.
//! Reads headers only; never executes binaries or maps whole files into memory.
use anyhow::{Result, bail, ensure};
use std::{
    collections::BTreeSet,
    io::{Read, Seek, SeekFrom},
};

/// MacPorts fileIsBinary classification for registry metadata, not structural
/// validation. Matches pextlib's native Mach-O, ar, and FAT_MAGIC header probe,
/// including its 1..20 architecture-count heuristic to exclude Java classes.
pub fn is_binary(reader: &mut impl Read) -> Result<bool> {
    fn word(reader: &mut impl Read) -> std::io::Result<Option<[u8; 4]>> {
        let mut bytes = [0; 4];
        match reader.read_exact(&mut bytes) {
            Ok(()) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
            Err(error) => Err(error),
        }
    }
    let Some(magic) = word(reader)? else {
        return Ok(false);
    };
    if matches!(u32::from_ne_bytes(magic), 0xfeedface | 0xfeedfacf) {
        return Ok(true);
    }
    if &magic == b"!<ar" {
        return Ok(word(reader)?.as_ref() == Some(b"ch>\n"));
    }
    if u32::from_be_bytes(magic) == 0xcafebabe {
        return Ok(word(reader)?.is_some_and(|count| (1..20).contains(&u32::from_be_bytes(count))));
    }
    Ok(false)
}

/// Return CPU architecture names, deduplicated and sorted. Non-Mach-O data
/// returns an empty set; malformed recognized containers return an error.
/// Install names from dylib load commands in every Mach-O slice, as
/// libmachista reports them. Non-Mach-O data yields no names.
pub fn linked_libraries(reader: &mut (impl Read + Seek)) -> Result<BTreeSet<String>> {
    let length = reader.seek(SeekFrom::End(0))?;
    let mut parser = Parser {
        reader,
        remaining: 1_000_000,
        cpus: BTreeSet::new(),
        libraries: BTreeSet::new(),
        slices: Vec::new(),
    };
    parser.parse(0, length, 0)?;
    Ok(parser.libraries)
}

/// A dylib load command (or the library's own `LC_ID_DYLIB`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dylib {
    pub command: u32,
    pub name: String,
    pub current: u32,
    pub compatibility: u32,
}
pub const LC_ID_DYLIB: u32 = 0xd;
pub const LC_LOAD_WEAK_DYLIB: u32 = 0x8000_0018;

/// One architecture of a Mach-O file, as libmachista reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slice {
    pub cpu: u32,
    pub id: Option<Dylib>,
    pub loads: Vec<Dylib>,
}
impl Slice {
    pub fn architecture(&self) -> Result<&'static str> {
        cpu_name(self.cpu)
    }
}

/// Slices of a Mach-O or fat file; `None` for other data, including static
/// archives, which libmachista rejects as not Mach-O.
pub fn slices(reader: &mut (impl Read + Seek)) -> Result<Option<Vec<Slice>>> {
    let length = reader.seek(SeekFrom::End(0))?;
    if length >= 8 {
        let mut magic = [0; 8];
        reader.seek(SeekFrom::Start(0))?;
        reader.read_exact(&mut magic)?;
        if &magic == b"!<arch>\n" {
            return Ok(None);
        }
    }
    let mut parser = Parser {
        reader,
        remaining: 1_000_000,
        cpus: BTreeSet::new(),
        libraries: BTreeSet::new(),
        slices: Vec::new(),
    };
    if !parser.parse(0, length, 0)? {
        return Ok(None);
    }
    Ok(Some(parser.slices))
}

/// Render a packed dylib version (xxxx.yy.zz) as libmachista does.
pub fn format_version(version: u32) -> String {
    let (major, minor, patch) = (version >> 16, (version >> 8) & 0xff, version & 0xff);
    if patch == 0 {
        format!("{major}.{minor}")
    } else {
        format!("{major}.{minor}.{patch}")
    }
}

fn cpu_name(cpu: u32) -> Result<&'static str> {
    Ok(match cpu {
        7 => "i386",
        0x0100_0007 => "x86_64",
        12 => "arm",
        0x0100_000c => "arm64",
        0x0200_000c => "arm64_32",
        18 => "ppc",
        0x0100_0012 => "ppc64",
        6 => "m68k",
        10 => "m98k",
        11 => "hppa",
        13 => "m88k",
        14 => "sparc",
        15 => "i860",
        _ => bail!("unknown Mach-O CPU type {cpu:#x}"),
    })
}

pub fn architectures(reader: &mut (impl Read + Seek)) -> Result<BTreeSet<String>> {
    let length = reader.seek(SeekFrom::End(0))?;
    let mut parser = Parser {
        reader,
        remaining: 1_000_000,
        cpus: BTreeSet::new(),
        libraries: BTreeSet::new(),
        slices: Vec::new(),
    };
    parser.parse(0, length, 0)?;
    parser
        .cpus
        .into_iter()
        .map(|cpu| {
            let name = match cpu {
                7 => "i386",
                0x0100_0007 => "x86_64",
                12 => "arm",
                0x0100_000c => "arm64",
                0x0200_000c => "arm64_32",
                18 => "ppc",
                0x0100_0012 => "ppc64",
                6 => "m68k",
                10 => "m98k",
                11 => "hppa",
                13 => "m88k",
                14 => "sparc",
                15 => "i860",
                _ => bail!("unknown Mach-O CPU type {cpu:#x}"),
            };
            Ok(name.to_owned())
        })
        .collect()
}

struct Parser<'a, R> {
    reader: &'a mut R,
    remaining: usize,
    cpus: BTreeSet<u32>,
    libraries: BTreeSet<String>,
    slices: Vec<Slice>,
}

/// LC_LOAD_DYLIB, LC_ID_DYLIB, LC_LOAD_WEAK_DYLIB, LC_REEXPORT_DYLIB,
/// LC_LAZY_LOAD_DYLIB and LC_LOAD_UPWARD_DYLIB.
const DYLIB_COMMANDS: [u32; 6] = [0xc, 0xd, 0x8000_0018, 0x8000_001f, 0x20, 0x8000_0023];
impl<R: Read + Seek> Parser<'_, R> {
    fn read<const N: usize>(&mut self, start: u64, end: u64) -> Result<[u8; N]> {
        ensure!(
            start <= end && N as u64 <= end - start,
            "truncated Mach-O container"
        );
        let mut bytes = [0; N];
        self.reader.seek(SeekFrom::Start(start))?;
        self.reader.read_exact(&mut bytes)?;
        Ok(bytes)
    }
    fn spend(&mut self) -> Result<()> {
        ensure!(self.remaining > 0, "Mach-O inspection work limit exceeded");
        self.remaining -= 1;
        Ok(())
    }
    fn parse(&mut self, start: u64, end: u64, depth: usize) -> Result<bool> {
        self.spend()?;
        ensure!(depth <= 32, "Mach-O container nesting limit exceeded");
        if end - start < 4 {
            return Ok(false);
        }
        let magic = u32::from_be_bytes(self.read(start, end)?);
        if magic == 0x213c6172 && end - start >= 8 && &self.read::<8>(start, end)? == b"!<arch>\n" {
            return self.archive(start, end, depth).map(|()| true);
        }
        match magic {
            0xfeedface | 0xcefaedfe | 0xfeedfacf | 0xcffaedfe => {
                let little = matches!(magic, 0xcefaedfe | 0xcffaedfe);
                let header_size = if matches!(magic, 0xfeedfacf | 0xcffaedfe) {
                    32
                } else {
                    28
                };
                let header = self.read::<28>(start, end)?;
                ensure!(end - start >= header_size, "truncated Mach-O header");
                let cpu = word(&header[4..8], little);
                let commands = word(&header[16..20], little);
                let size = word(&header[20..24], little) as u64;
                let mut cursor = start + header_size;
                let mut slice = Slice {
                    cpu,
                    id: None,
                    loads: Vec::new(),
                };
                ensure!(size <= end - cursor, "truncated Mach-O load commands");
                let limit = cursor + size;
                ensure!(
                    u64::from(commands) <= size / 8,
                    "invalid Mach-O load command count"
                );
                for _ in 0..commands {
                    self.spend()?;
                    let command = self.read::<8>(cursor, limit)?;
                    let length = word(&command[4..], little) as u64;
                    ensure!(
                        length >= 8 && length <= limit - cursor,
                        "invalid Mach-O load command size"
                    );
                    let kind = word(&command[..4], little);
                    if DYLIB_COMMANDS.contains(&kind) && length >= 12 {
                        let offset = word(&self.read::<4>(cursor + 8, limit)?, little) as u64;
                        if offset >= 12 && offset < length {
                            let start = cursor + offset;
                            let size = (cursor + length - start) as usize;
                            let mut name = vec![0; size];
                            self.reader.seek(SeekFrom::Start(start))?;
                            self.reader.read_exact(&mut name)?;
                            let end = name.iter().position(|b| *b == 0).unwrap_or(name.len());
                            let name = String::from_utf8_lossy(&name[..end]).into_owned();
                            self.libraries.insert(name.clone());
                            let (current, compatibility) = if length >= 24 && offset >= 24 {
                                let versions = self.read::<8>(cursor + 16, limit)?;
                                (word(&versions[..4], little), word(&versions[4..], little))
                            } else {
                                (0, 0)
                            };
                            let dylib = Dylib {
                                command: kind,
                                name,
                                current,
                                compatibility,
                            };
                            if kind == LC_ID_DYLIB {
                                slice.id = Some(dylib);
                            } else {
                                slice.loads.push(dylib);
                            }
                        }
                    }
                    cursor += length;
                }
                self.cpus.insert(cpu);
                self.slices.push(slice);
            }
            0xcafebabe | 0xbebafeca | 0xcafebabf | 0xbfbafeca => {
                let little = matches!(magic, 0xbebafeca | 0xbfbafeca);
                let wide = matches!(magic, 0xcafebabf | 0xbfbafeca);
                let header = self.read::<8>(start, end)?;
                let count = word(&header[4..], little) as u64;
                let entry_size = if wide { 32 } else { 20 };
                ensure!(
                    count <= (end - start - 8) / entry_size,
                    "truncated Mach-O fat table"
                );
                for index in 0..count {
                    self.spend()?;
                    let offset = start + 8 + index * entry_size;
                    let (relative, length) = if wide {
                        let entry = self.read::<32>(offset, end)?;
                        (
                            wide_word(&entry[8..16], little),
                            wide_word(&entry[16..24], little),
                        )
                    } else {
                        let entry = self.read::<20>(offset, end)?;
                        (
                            word(&entry[8..12], little) as u64,
                            word(&entry[12..16], little) as u64,
                        )
                    };
                    ensure!(
                        relative >= 8 + count * entry_size
                            && relative <= end - start
                            && length <= end - start - relative,
                        "invalid Mach-O fat slice range"
                    );
                    ensure!(
                        self.parse(start + relative, start + relative + length, depth + 1)?,
                        "non-Mach-O fat slice"
                    );
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn archive(&mut self, start: u64, end: u64, depth: usize) -> Result<()> {
        let mut cursor = start + 8;
        while cursor < end {
            self.spend()?;
            let header = self.read::<60>(cursor, end)?;
            ensure!(&header[58..] == b"`\n", "invalid ar member header");
            let size = decimal(&header[48..58])?;
            cursor += 60;
            ensure!(size <= end - cursor, "truncated ar member");
            let name_length = if &header[..3] == b"#1/" {
                decimal(&header[3..16])?
            } else {
                0
            };
            ensure!(name_length <= size, "invalid ar extended name length");
            self.parse(cursor + name_length, cursor + size, depth + 1)?;
            cursor += size;
            if !(cursor - start).is_multiple_of(2) {
                ensure!(cursor < end, "missing ar member padding");
                cursor += 1;
            }
        }
        Ok(())
    }
}
fn word(bytes: &[u8], little: bool) -> u32 {
    let bytes = bytes.try_into().unwrap();
    if little {
        u32::from_le_bytes(bytes)
    } else {
        u32::from_be_bytes(bytes)
    }
}
fn wide_word(bytes: &[u8], little: bool) -> u64 {
    let bytes = bytes.try_into().unwrap();
    if little {
        u64::from_le_bytes(bytes)
    } else {
        u64::from_be_bytes(bytes)
    }
}
fn decimal(bytes: &[u8]) -> Result<u64> {
    Ok(std::str::from_utf8(bytes)?.trim().parse()?)
}
