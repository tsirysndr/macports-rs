//! Native lsearch modes used by Portfiles and archive resource definitions.
use crate::{
    check_args, glob::Glob, molt_err, molt_ok, ContextID, Exception, Interp, MoltInt, MoltResult,
    Value,
};

const OPTIONS: &[&str] = &[
    "-all",
    "-ascii",
    "-bisect",
    "-decreasing",
    "-dictionary",
    "-exact",
    "-glob",
    "-increasing",
    "-index",
    "-inline",
    "-integer",
    "-nocase",
    "-not",
    "-real",
    "-regexp",
    "-sorted",
    "-start",
    "-stride",
    "-subindices",
];

pub fn command(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 0, "?-option value ...? list pattern")?;
    let mut all = false;
    let mut inline = false;
    let mut negated = false;
    let mut nocase = false;
    let mut exact = false;
    let mut numeric = "-ascii";
    let mut start = None;
    let mut offset = 1;
    while offset < argv.len() - 2 {
        let option = argv[offset].as_str();
        let selected = if let Some(exact) = OPTIONS.iter().find(|name| **name == option) {
            *exact
        } else {
            let mut choices = OPTIONS.iter().filter(|name| name.starts_with(option));
            match (choices.next(), choices.next()) {
                (Some(name), None) => *name,
                _ => return molt_err!("bad or ambiguous option \"{}\"", option),
            }
        };
        match selected {
            "-all" => all = true,
            "-inline" => inline = true,
            "-not" => negated = true,
            "-nocase" => nocase = true,
            "-exact" => exact = true,
            "-glob" => exact = false,
            "-ascii" | "-integer" | "-real" => numeric = selected,
            "-increasing" | "-decreasing" => (), // Ordering affects sorted modes only.
            "-start" => {
                offset += 1;
                if offset >= argv.len() - 2 {
                    return molt_err!("missing starting index");
                }
                start = Some(&argv[offset]);
            }
            _ => return molt_err!("lsearch {} is not implemented", selected),
        }
        offset += 1;
    }
    let list = argv[argv.len() - 2].as_list()?;
    let explicit_start = start.is_some();
    let start = match start {
        Some(start) => super::list_ops::index(start, list.len())?.max(0) as usize,
        None => 0,
    };
    if explicit_start && start >= list.len() {
        return if all || inline {
            molt_ok!()
        } else {
            molt_ok!(-1)
        };
    }
    let pattern = &argv[argv.len() - 1];
    let glob = Glob::new(pattern.as_str(), !nocase);
    let integer = if exact && numeric == "-integer" {
        Some(pattern.as_int()?)
    } else {
        None
    };
    let real = if exact && numeric == "-real" {
        Some(pattern.as_float()?)
    } else {
        None
    };
    let mut results = Vec::new();
    for (index, value) in list.iter().enumerate().skip(start) {
        let matched = if let Some(integer) = integer {
            value.as_int()? == integer
        } else if let Some(real) = real {
            value.as_float()? == real
        } else if exact && nocase {
            value
                .as_str()
                .chars()
                .map(lower)
                .eq(pattern.as_str().chars().map(lower))
        } else if exact {
            value.as_str() == pattern.as_str()
        } else {
            glob.matches(value.as_str())
        };
        if matched != negated {
            let result = if inline {
                value.clone()
            } else {
                Value::from(index as MoltInt)
            };
            if !all {
                return Ok(result);
            }
            results.push(result);
        }
    }
    if all {
        molt_ok!(Value::from(results))
    } else if inline {
        molt_ok!()
    } else {
        molt_ok!(-1)
    }
}
fn lower(value: char) -> char {
    value.to_lowercase().next().unwrap()
}
