//! Bounded, seek-based architecture inspection of Mach-O and Unix ar containers.
//! Reads headers only; never executes binaries or maps whole files into memory.
use anyhow::{Result, bail, ensure};
use std::{
    collections::BTreeSet,
    io::{Read, Seek, SeekFrom},
};

/// Return CPU architecture names, deduplicated and sorted. Non-Mach-O data
/// returns an empty set; malformed recognized containers return an error.
pub fn architectures(reader: &mut (impl Read + Seek)) -> Result<BTreeSet<String>> {
    let length = reader.seek(SeekFrom::End(0))?;
    let mut parser = Parser {
        reader,
        remaining: 1_000_000,
        cpus: BTreeSet::new(),
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
}
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
                    cursor += length;
                }
                self.cpus.insert(cpu);
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
