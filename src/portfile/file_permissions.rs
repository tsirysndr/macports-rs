//! Unix permission attributes used by work-directory and staging hooks.
use super::*;
use molt::{MoltResult, check_args};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
};

pub(super) fn command(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(2, argv, 3, 0, "name ?option? ?value option value ...?")?;
    let setter = argv.len() > 4;
    if argv.len() == 3 || setter && interp.context::<State>(id).system_executor.is_none() {
        return unsupported(interp, id);
    }
    let path = argv[2].as_str();
    let mut index = 3;
    while index < argv.len() {
        let option = argv[index].as_str();
        if !option.is_empty()
            && option != "-"
            && let Some(group) = if "-owner".starts_with(option) {
                Some(false)
            } else if "-group".starts_with(option) {
                Some(true)
            } else {
                None
            }
        {
            let value = if setter {
                Some(
                    argv.get(index + 1)
                        .ok_or_else(|| error(format!("value for \"{option}\" missing")))?,
                )
            } else {
                None
            };
            let result = file_ownership::attribute(path, group, value)?;
            if !setter {
                return Ok(result);
            }
            index += 2;
            continue;
        }
        // Other attributes must not silently yield incomplete metadata.
        if option.is_empty() || !"-permissions".starts_with(option) || option == "-" {
            return unsupported(interp, id);
        }
        if !setter {
            let metadata =
                fs::metadata(path).map_err(|err| filesystem::file_error(path, false, err))?;
            return Ok(format!("0{:04o}", metadata.mode() & 0x7fff).into());
        }
        let value = argv
            .get(index + 1)
            .ok_or_else(|| error(format!("value for \"{option}\" missing")))?;
        let mode = if let Some(mode) = numeric(value.as_str()) {
            mode
        } else {
            let metadata =
                fs::metadata(path).map_err(|err| filesystem::file_error(path, false, err))?;
            symbolic(value.as_str(), metadata.mode() & 0x7fff).ok_or_else(|| {
                Exception::molt_err2(
                    Value::from(vec!["TCL".into(), "VALUE".into(), "PERMISSION".into()]),
                    format!("unknown permission string format \"{value}\"").into(),
                )
            })?
        };
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|err| {
            filesystem::path_error(path, "could not set permissions for file", err)
        })?;
        index += 2;
    }
    Ok(Value::empty())
}
fn unsupported(interp: &mut Interp, id: ContextID) -> MoltResult {
    interp
        .context::<State>(id)
        .unsupported
        .insert("file attributes".into());
    Err(error("unsupported Portfile command file attributes"))
}
fn numeric(text: &str) -> Option<u32> {
    let text = text.trim();
    // Tcl's permission setter retains legacy leading-zero octal independently
    // of Tcl 9's normal decimal integer syntax.
    if text.starts_with('0')
        && text
            .as_bytes()
            .get(1)
            .is_some_and(|b| matches!(b, b'0'..=b'7'))
        && let Ok(value) = i64::from_str_radix(&text[1..], 8)
    {
        return Some((value & 0x7fff) as u32);
    }
    Value::get_int(text)
        .ok()
        .map(|number| (number & 0x7fff) as u32)
}
fn symbolic(text: &str, mut mode: u32) -> Option<u32> {
    if let Some(value) = rwx(text) {
        return Some(value);
    }
    let bytes = text.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let mut who = 0;
        while let Some(byte) = bytes.get(cursor) {
            who |= match byte {
                b'u' => 0o4700,
                b'g' => 0o2070,
                b'o' => 0o1007,
                b'a' => 0o7777,
                _ => break,
            };
            cursor += 1;
        }
        // Tcl accepts an empty string or a trailing who-only clause unchanged.
        let Some(&op) = bytes.get(cursor) else {
            break;
        };
        if !matches!(op, b'+' | b'-' | b'=') {
            return None;
        }
        if who == 0 {
            who = 0o7777;
        }
        cursor += 1;
        let mut what = 0;
        while let Some(&byte) = bytes.get(cursor) {
            cursor += 1;
            what |= match byte {
                b'r' => 0o444,
                b'w' => 0o222,
                b'x' => 0o111,
                b's' => 0o6000,
                b't' => 0o1000,
                b',' => break,
                _ => return None,
            };
        }
        mode = match op {
            b'+' => mode | (who & what),
            b'-' => mode & !(who & what),
            b'=' => (mode & !who) | (who & what),
            _ => unreachable!(),
        };
    }
    Some(mode)
}
fn rwx(text: &str) -> Option<u32> {
    if text.len() != 9 {
        return None;
    }
    let mut mode = 0;
    for (i, byte) in text.bytes().enumerate() {
        match byte {
            b'-' => (),
            b'r' if i % 3 == 0 => mode |= 1 << (8 - i),
            b'w' if i % 3 == 1 => mode |= 1 << (8 - i),
            b'x' if i % 3 == 2 => mode |= 1 << (8 - i),
            b's' | b'S' if i == 2 || i == 5 => {
                mode |= 1 << (11 - i / 3);
                if byte == b's' {
                    mode |= 1 << (8 - i);
                }
            }
            b't' | b'T' if i == 8 => {
                mode |= 0o1000;
                if byte == b't' {
                    mode |= 1;
                }
            }
            _ => return None,
        }
    }
    Some(mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn special_bits_and_numeric_masking_follow_tcl() {
        assert_eq!(symbolic("rwsr-Sr-t", 0), Some(0o7745));
        assert_eq!(symbolic("rwSr--r-T", 0), Some(0o5644));
        assert_eq!(symbolic("a=rwx,u+s,g+s,o+t", 0), Some(0o7777));
        assert_eq!(symbolic("u=rw", 0o7777), Some(0o3677));
        assert_eq!(numeric(" 0755 "), Some(0o755));
        assert_eq!(numeric("-1"), Some(0x7fff));
        assert_eq!(numeric("65535"), Some(0x7fff));
        assert_eq!(numeric("++755"), None);
        assert_eq!(numeric("0o+755"), None);
    }
}
