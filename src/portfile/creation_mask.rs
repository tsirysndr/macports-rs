//! Session-local BSD umask semantics; never mutate the interpreter process mask.
use super::*;
use molt::{MoltResult, check_args};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("umask", command, id);
}
impl Session {
    /// Override the captured startup mask, or supply a mask when the embedding
    /// host did not capture one. Native umask commands and child requests use it.
    pub fn initialize_creation_mask(&mut self, mask: u32) -> Result<()> {
        ensure!(
            mask <= 0o777,
            "creation mask must contain only permission bits"
        );
        self.interp.context::<State>(self.id).creation_mask = Some(mask);
        Ok(())
    }
}
fn command(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "mode")?;
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    let old = interp
        .context::<State>(id)
        .creation_mask
        .ok_or_else(|| error("creation mask is not initialized"))?;
    let mask = parse(argv[1].as_str(), old).ok_or_else(|| error("Invalid umask mode"))?;
    interp.context::<State>(id).creation_mask = Some(mask & 0o777);
    Ok(format!("{old:04o}").into())
}
const STANDARD: u32 = 0o6777;
fn apply(mode: &mut u32, op: u8, who: u32, bits: u32, allowed: u32) {
    if op == b'=' {
        *mode &= !if who == 0 { STANDARD } else { who };
    }
    let bits = bits & if who == 0 { allowed } else { who };
    if op == b'-' {
        *mode &= !bits;
    } else {
        *mode |= bits;
    }
}
fn copy(mode: &mut u32, op: u8, who: u32, from: u8, allowed: u32) {
    let value = (*mode
        >> match from {
            b'u' => 6,
            b'g' => 3,
            _ => 0,
        })
        & 7;
    for shift in [0, 3, 6] {
        if who != 0 && who & (0o4 << shift) == 0 {
            continue;
        }
        let filter = if who == 0 { allowed } else { u32::MAX };
        if op != b'+' {
            *mode &= !((if op == b'=' { 7 } else { value } << shift) & filter);
        }
        if op != b'-' {
            *mode |= (value << shift) & filter;
        }
    }
}
/// BSD setmode/getmode applied to an initial mode of zero, as in Pextlib.
fn parse(text: &str, old: u32) -> Option<u32> {
    let bytes = text.as_bytes();
    if bytes.first()?.is_ascii_digit() {
        if !bytes.iter().all(|b| matches!(b, b'0'..=b'7')) {
            return None;
        }
        let value = u32::from_str_radix(text, 8).ok()?;
        return (value <= 0o7777).then_some(value);
    }
    let (mut cursor, mut mode) = (0, 0);
    let allowed = !old;
    loop {
        let mut who = 0;
        while let Some(byte) = bytes.get(cursor) {
            who |= match byte {
                b'a' => STANDARD,
                b'u' => 0o4700,
                b'g' => 0o2070,
                b'o' => 0o7,
                _ => break,
            };
            cursor += 1;
        }
        loop {
            let op = *bytes.get(cursor)?;
            if !matches!(op, b'+' | b'-' | b'=') {
                return None;
            }
            cursor += 1;
            who &= !0o1000;
            let (mut bits, mut equal_done) = (0, false);
            while let Some(&byte) = bytes.get(cursor) {
                match byte {
                    b'r' => bits |= 0o444,
                    b'w' => bits |= 0o222,
                    b'x' => bits |= 0o111,
                    // getmode's X checks the original mode (zero), not results
                    // from earlier clauses, so it never sets bits here.
                    b'X' => (),
                    b's' => {
                        if who == 0 || who & !7 != 0 {
                            bits |= 0o6000;
                        }
                    }
                    b't' => {
                        if who == 0 || who & !7 != 0 {
                            who |= 0o1000;
                            bits |= 0o1000;
                        }
                    }
                    b'u' | b'g' | b'o' => {
                        if bits != 0 {
                            apply(&mut mode, op, who, bits, allowed);
                            bits = 0;
                        }
                        if op == b'=' {
                            equal_done = true;
                        }
                        copy(&mut mode, op, who, byte, allowed);
                    }
                    _ => break,
                }
                cursor += 1;
            }
            if bits != 0 || (op == b'=' && !equal_done) {
                apply(&mut mode, op, who, bits, allowed);
            }
            match bytes.get(cursor) {
                None => return Some(mode),
                Some(b',') => {
                    cursor += 1;
                    break;
                }
                Some(b'+' | b'-' | b'=') => (),
                _ => return None,
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_bsd_octal_symbolic_and_copy_modes() {
        for (input, expected) in [
            ("022", 0o22),
            ("07777", 0o7777),
            ("u=rwx,g=rx,o=", 0o750),
            ("u=rw,g=u,o=g", 0o666),
            ("u+x,g+X", 0o100),
            ("+w", 0o200),
            ("a+w", 0o222),
            ("u=rw-w", 0o400),
            ("u=rwg", 0),
            ("=rw", 0o644),
            ("u=rw,g=u-w", 0o640),
            ("u=", 0),
            ("o+s", 0),
            ("a+t", 0o1000),
        ] {
            assert_eq!(parse(input, 0o022), Some(expected), "{input}");
        }
        for input in [
            "", "08", "0o22", "10000", " 022", "u", "u+q", "u=r,", "u=r,,g=w",
        ] {
            assert_eq!(parse(input, 0o022), None, "{input}");
        }
    }
}
