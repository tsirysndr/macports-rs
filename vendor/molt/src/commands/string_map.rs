//! Ordered, single-pass substitutions used by PortGroups to construct procedures.
use crate::{check_args, molt_err, molt_ok, ContextID, Exception, Interp, MoltResult, Value};

pub(super) fn command(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(2, argv, 4, 5, "?-nocase? charMap string")?;
    let nocase = argv.len() == 5;
    if nocase {
        let option = argv[2].as_str();
        if option.len() <= 1 || !"-nocase".starts_with(option) {
            return molt_err!("bad option \"{}\": must be -nocase", option);
        }
    }
    let mapping = argv[argv.len() - 2].as_list()?;
    let (pairs, remainder) = mapping.as_chunks::<2>();
    if !remainder.is_empty() {
        return molt_err!("char map list unbalanced");
    }
    let source = &argv[argv.len() - 1];
    if mapping.is_empty() {
        return Ok(source.clone());
    }
    // Retain list order and duplicate keys. Empty keys never match. Fold keys
    // once, without expanding characters (Tcl uses simple Unicode lowercase).
    let pairs: Vec<_> = pairs
        .iter()
        .filter(|pair| !pair[0].as_str().is_empty())
        .map(|pair| {
            (
                pair[0].as_str(),
                pair[1].as_str(),
                if nocase {
                    pair[0].as_str().chars().map(lower).collect::<Vec<_>>()
                } else {
                    Vec::new()
                },
            )
        })
        .collect();
    let mut remaining = source.as_str();
    let mut result = String::with_capacity(remaining.len());
    while !remaining.is_empty() {
        let replacement = pairs.iter().find_map(|(key, value, folded)| {
            let length = if nocase {
                prefix_length(remaining, folded)?
            } else if remaining.starts_with(key) {
                key.len()
            } else {
                return None;
            };
            Some((length, *value))
        });
        if let Some((length, value)) = replacement {
            result.push_str(value);
            remaining = &remaining[length..];
        } else {
            let length = remaining.chars().next().unwrap().len_utf8();
            result.push_str(&remaining[..length]);
            remaining = &remaining[length..];
        }
    }
    molt_ok!(result)
}

fn lower(c: char) -> char {
    // Tcl 8.6's UTF-16 string representation compares supplementary characters
    // as surrogate pairs, whose units have no case mappings.
    if c as u32 > 0xffff {
        c
    } else {
        c.to_lowercase().next().unwrap()
    }
}

fn prefix_length(source: &str, key: &[char]) -> Option<usize> {
    let mut source = source.chars();
    let mut bytes = 0;
    for expected in key {
        let actual = source.next()?;
        if lower(actual) != *expected {
            return None;
        }
        bytes += actual.len_utf8();
    }
    Some(bytes)
}
