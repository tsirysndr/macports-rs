//! Core Tcl commands used by MacPorts Portfiles and PortGroups: control flow
//! (`switch`, `eval`, `uplevel`, `subst`), list utilities, `format`, `glob`,
//! and the remaining `string` subcommands.
use super::list_ops;
use crate::glob::Glob;
use crate::{
    check_args, molt_err, molt_ok, ContextID, Exception, Interp, MoltInt, MoltResult, Value,
};
use std::cmp::Ordering;
use std::convert::TryFrom;

fn concat(values: &[Value]) -> String {
    values
        .iter()
        .map(|value| value.as_str().trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn script_of(values: &[Value]) -> Value {
    if values.len() == 1 {
        values[0].clone()
    } else {
        Value::from(concat(values))
    }
}

/// # concat ?arg ...?
pub fn cmd_concat(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    molt_ok!(concat(&argv[1..]))
}

/// # eval arg ?arg ...?
pub fn cmd_eval(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 0, "arg ?arg ...?")?;
    interp.eval_value(&script_of(&argv[1..]))
}

fn level_of(interp: &Interp, text: &str) -> Option<usize> {
    let current = interp.scope_level();
    if let Some(absolute) = text.strip_prefix('#') {
        absolute
            .parse::<usize>()
            .ok()
            .filter(|level| *level <= current)
    } else {
        let relative = text.parse::<usize>().ok()?;
        current.checked_sub(relative)
    }
}

/// # uplevel ?level? arg ?arg ...?
pub fn cmd_uplevel(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 0, "?level? command ?arg ...?")?;
    let first = argv[1].as_str();
    let looks_like_level = first.starts_with('#') || first.parse::<usize>().is_ok();
    let (level, script) = if looks_like_level && argv.len() > 2 {
        match level_of(interp, first) {
            Some(level) => (level, &argv[2..]),
            None => return molt_err!("bad level \"{}\"", first),
        }
    } else {
        match level_of(interp, "1") {
            Some(level) => (level, &argv[1..]),
            None => return molt_err!("bad level \"1\""),
        }
    };
    let script = script_of(script);
    interp.with_scope_level(level, |interp| interp.eval_value(&script))
}

/// # subst ?-nobackslashes? ?-nocommands? ?-novariables? string
pub fn cmd_subst(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(
        1,
        argv,
        2,
        5,
        "?-nobackslashes? ?-nocommands? ?-novariables? string",
    )?;
    let text = argv[argv.len() - 1].as_str();
    let (mut backslashes, mut commands, mut variables) = (true, true, true);
    for flag in &argv[1..argv.len() - 1] {
        match flag.as_str() {
            "-nobackslashes" => backslashes = false,
            "-nocommands" => commands = false,
            "-novariables" => variables = false,
            other => return molt_err!("bad switch \"{}\"", other),
        }
    }
    // Disable a substitution class by escaping its introducers first.
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        let protect = (!backslashes && character == '\\')
            || (!commands && character == '[')
            || (!variables && character == '$');
        if protect {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    let word = crate::parser::parse_subst(&escaped)?;
    interp.eval_word(&word)
}

/// # switch ?options? string pattern body ?pattern body ...?
pub fn cmd_switch(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(
        1,
        argv,
        3,
        0,
        "?-option ...? string ?pattern body ...? ?default body?",
    )?;
    #[derive(PartialEq)]
    enum Mode {
        Exact,
        Glob,
        Regexp,
    }
    let mut mode = Mode::Exact;
    let mut nocase = false;
    let mut index = 1;
    while index < argv.len() - 2 {
        match argv[index].as_str() {
            "-exact" => mode = Mode::Exact,
            "-glob" => mode = Mode::Glob,
            "-regexp" => mode = Mode::Regexp,
            "-nocase" => nocase = true,
            "--" => {
                index += 1;
                break;
            }
            _ => break,
        }
        index += 1;
    }
    let subject = argv[index].as_str().to_string();
    index += 1;
    let pairs: Vec<Value> = if argv.len() - index == 1 {
        argv[index].as_list()?.to_vec()
    } else {
        argv[index..].to_vec()
    };
    if !pairs.len().is_multiple_of(2) {
        return molt_err!("extra switch pattern with no body");
    }
    let last = pairs.len() / 2 - 1;
    let mut matched = None;
    for (position, pair) in pairs.chunks(2).enumerate() {
        let pattern = pair[0].as_str();
        let hit = if position == last && pattern == "default" {
            true
        } else {
            match mode {
                Mode::Exact if nocase => pattern.to_lowercase() == subject.to_lowercase(),
                Mode::Exact => pattern == subject,
                Mode::Glob => Glob::new(pattern, !nocase).matches(&subject),
                Mode::Regexp => {
                    super::regexp::compile(pattern, nocase, false, false)?.is_match(&subject)
                }
            }
        };
        if hit {
            matched = Some(position);
            break;
        }
    }
    let Some(position) = matched else {
        return molt_ok!();
    };
    for pair in pairs.chunks(2).skip(position) {
        if pair[1].as_str() != "-" {
            return interp.eval_value(&pair[1]);
        }
    }
    molt_err!("no body specified for pattern \"{}\"", pairs[position * 2])
}

/// # lassign list ?varName ...?
pub fn cmd_lassign(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 0, "list ?varName ...?")?;
    let list = argv[1].as_list()?;
    for (position, name) in argv[2..].iter().enumerate() {
        let value = list.get(position).cloned().unwrap_or_else(Value::empty);
        interp.set_var(name, value)?;
    }
    let rest: Vec<Value> = list.iter().skip(argv.len() - 2).cloned().collect();
    molt_ok!(Value::from(rest))
}

fn clamp(index: MoltInt, length: usize) -> usize {
    index.clamp(0, length as MoltInt) as usize
}

/// # lreplace list first last ?element ...?
pub fn cmd_lreplace(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 4, 0, "list first last ?element ...?")?;
    let list = argv[1].as_list()?.to_vec();
    let length = list.len();
    let first = clamp(list_ops::index(&argv[2], length)?, length);
    let last = list_ops::index(&argv[3], length)?;
    let end = if last < first as MoltInt {
        first
    } else {
        clamp(last + 1, length)
    };
    let mut result = list[..first].to_vec();
    result.extend_from_slice(&argv[4..]);
    result.extend_from_slice(&list[end.max(first)..]);
    molt_ok!(Value::from(result))
}

/// # linsert list index ?element ...?
pub fn cmd_linsert(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 0, "list index ?element ...?")?;
    let mut list = argv[1].as_list()?.to_vec();
    let length = list.len();
    let text = argv[2].as_str();
    // `end` inserts after the last element, unlike most index uses.
    let position = if text == "end" {
        length
    } else if text.starts_with("end") {
        clamp(list_ops::index(&argv[2], length)? + 1, length)
    } else {
        clamp(list_ops::index(&argv[2], length)?, length)
    };
    for (offset, element) in argv[3..].iter().enumerate() {
        list.insert(position + offset, element.clone());
    }
    molt_ok!(Value::from(list))
}

/// # lreverse list
pub fn cmd_lreverse(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "list")?;
    let mut list = argv[1].as_list()?.to_vec();
    list.reverse();
    molt_ok!(Value::from(list))
}

/// # lrepeat count ?element ...?
pub fn cmd_lrepeat(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 0, "count ?value ...?")?;
    let count = argv[1].as_int()?;
    if count < 0 {
        return molt_err!("bad count \"{}\": must be integer >= 0", count);
    }
    let mut list = Vec::new();
    for _ in 0..count {
        list.extend_from_slice(&argv[2..]);
    }
    molt_ok!(Value::from(list))
}

fn set_path(list: &Value, path: &[Value], value: Value) -> Result<Value, Exception> {
    let Some((first, rest)) = path.split_first() else {
        return Ok(value);
    };
    let mut items = list.as_list()?.to_vec();
    let index = list_ops::index(first, items.len())?;
    if index < 0 || index as usize > items.len() {
        return molt_err!("list index out of range");
    }
    let index = index as usize;
    if index == items.len() {
        items.push(set_path(&Value::empty(), rest, value)?);
    } else {
        items[index] = set_path(&items[index], rest, value)?;
    }
    Ok(Value::from(items))
}

/// # lset varName ?index ...? value
pub fn cmd_lset(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 0, "listVar ?index? ?index ...? value")?;
    let current = interp.var(&argv[1])?;
    let indices: Vec<Value> = if argv.len() == 4 {
        argv[2].as_list()?.to_vec()
    } else {
        argv[2..argv.len() - 1].to_vec()
    };
    let updated = set_path(&current, &indices, argv[argv.len() - 1].clone())?;
    interp.set_var_return(&argv[1], updated)
}

/// Tcl `-dictionary` ordering: case-insensitive, with embedded numbers compared numerically.
pub fn dictionary_compare(a: &str, b: &str) -> Ordering {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (mut i, mut j) = (0, 0);
    let mut tie = Ordering::Equal;
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let start = (i, j);
            while i < a.len() && a[i] == '0' {
                i += 1;
            }
            while j < b.len() && b[j] == '0' {
                j += 1;
            }
            let (ni, nj) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let order = (i - ni)
                .cmp(&(j - nj))
                .then_with(|| a[ni..i].cmp(&b[nj..j]));
            if order != Ordering::Equal {
                return order;
            }
            if tie == Ordering::Equal {
                tie = (ni - start.0).cmp(&(nj - start.1)).reverse();
            }
            continue;
        }
        let (x, y) = (
            a[i].to_lowercase().next().unwrap(),
            b[j].to_lowercase().next().unwrap(),
        );
        if x != y {
            return x.cmp(&y);
        }
        if tie == Ordering::Equal && a[i] != b[j] {
            tie = if a[i].is_uppercase() {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        i += 1;
        j += 1;
    }
    (a.len() - i).cmp(&(b.len() - j)).then(tie)
}

/// # lsort ?options? list
pub fn cmd_lsort(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 0, "?-option value ...? list")?;
    #[derive(Clone, Copy, PartialEq)]
    enum Kind {
        Ascii,
        Dictionary,
        Integer,
        Real,
    }
    let mut kind = Kind::Ascii;
    let (mut decreasing, mut unique, mut nocase) = (false, false, false);
    let mut index: Option<Value> = None;
    let mut command: Option<Value> = None;
    let mut position = 1;
    const LSORT_OPTIONS: &[&str] = &[
        "-ascii",
        "-dictionary",
        "-integer",
        "-real",
        "-increasing",
        "-decreasing",
        "-unique",
        "-nocase",
        "-index",
        "-command",
    ];
    while position < argv.len() - 1 {
        match option_prefix(argv[position].as_str(), LSORT_OPTIONS) {
            "-ascii" => kind = Kind::Ascii,
            "-dictionary" => kind = Kind::Dictionary,
            "-integer" => kind = Kind::Integer,
            "-real" => kind = Kind::Real,
            "-increasing" => decreasing = false,
            "-decreasing" => decreasing = true,
            "-unique" => unique = true,
            "-nocase" => nocase = true,
            "-index" => {
                position += 1;
                index = argv.get(position).cloned();
            }
            "-command" => {
                position += 1;
                command = argv.get(position).cloned();
            }
            other => return molt_err!("bad option \"{}\"", other),
        }
        position += 1;
    }
    let list = argv[argv.len() - 1].as_list()?.to_vec();
    let mut keyed = Vec::with_capacity(list.len());
    for item in list {
        let key = match &index {
            Some(index) => {
                let elements = item.as_list()?;
                let position = list_ops::index(index, elements.len())?;
                match usize::try_from(position).ok().and_then(|p| elements.get(p)) {
                    Some(key) => key.clone(),
                    None => {
                        return molt_err!("element {} missing from sublist \"{}\"", index, item)
                    }
                }
            }
            None => item.clone(),
        };
        keyed.push((key, item));
    }
    let failure = std::cell::RefCell::new(None);
    let mut compare = |a: &Value, b: &Value| -> Ordering {
        if failure.borrow().is_some() {
            return Ordering::Equal;
        }
        let order = if let Some(command) = &command {
            let mut words = command.as_list().map(|l| l.to_vec()).unwrap_or_default();
            words.push(a.clone());
            words.push(b.clone());
            match interp
                .eval_value(&Value::from(words))
                .and_then(|v| v.as_int())
            {
                Ok(value) => value.cmp(&0),
                Err(error) => {
                    *failure.borrow_mut() = Some(error);
                    Ordering::Equal
                }
            }
        } else {
            match kind {
                Kind::Ascii if nocase => a.as_str().to_lowercase().cmp(&b.as_str().to_lowercase()),
                Kind::Ascii => a.as_str().cmp(b.as_str()),
                Kind::Dictionary => dictionary_compare(a.as_str(), b.as_str()),
                Kind::Integer => match (a.as_int(), b.as_int()) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    (Err(error), _) | (_, Err(error)) => {
                        *failure.borrow_mut() = Some(error);
                        Ordering::Equal
                    }
                },
                Kind::Real => match (a.as_float(), b.as_float()) {
                    (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
                    (Err(error), _) | (_, Err(error)) => {
                        *failure.borrow_mut() = Some(error);
                        Ordering::Equal
                    }
                },
            }
        };
        if decreasing {
            order.reverse()
        } else {
            order
        }
    };
    // A stable merge sort keeps equal elements in input order, as Tcl does.
    keyed.sort_by(|(a, _), (b, _)| compare(a, b));
    if unique {
        let mut deduplicated: Vec<(Value, Value)> = Vec::new();
        for entry in keyed {
            match deduplicated.last_mut() {
                Some(last) if compare(&last.0, &entry.0) == Ordering::Equal => *last = entry,
                _ => deduplicated.push(entry),
            }
        }
        keyed = deduplicated;
    }
    if let Some(error) = failure.into_inner() {
        return Err(error);
    }
    molt_ok!(Value::from(
        keyed.into_iter().map(|(_, item)| item).collect::<Vec<_>>()
    ))
}

// ---------------------------------------------------------------------------
// format

/// # format formatString ?arg ...?
pub fn cmd_format(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 0, "formatString ?arg ...?")?;
    let spec: Vec<char> = argv[1].as_str().chars().collect();
    let args = &argv[2..];
    let mut next = 0;
    let mut output = String::new();
    let mut i = 0;
    while i < spec.len() {
        if spec[i] != '%' {
            output.push(spec[i]);
            i += 1;
            continue;
        }
        i += 1;
        if spec.get(i) == Some(&'%') {
            output.push('%');
            i += 1;
            continue;
        }
        // Optional XPG3 position: %n$
        let start = i;
        while i < spec.len() && spec[i].is_ascii_digit() {
            i += 1;
        }
        if spec.get(i) == Some(&'$') && i > start {
            let position: usize = spec[start..i].iter().collect::<String>().parse().unwrap();
            next = position.saturating_sub(1);
            i += 1;
        } else {
            i = start;
        }
        let mut flags = String::new();
        while i < spec.len() && "-+ 0#".contains(spec[i]) {
            flags.push(spec[i]);
            i += 1;
        }
        let take = |next: &mut usize| -> Result<Value, Exception> {
            let value = args.get(*next).cloned();
            *next += 1;
            value.ok_or_else(|| {
                Exception::molt_err(Value::from(
                    "not enough arguments for all format specifiers",
                ))
            })
        };
        let mut width = None;
        if spec.get(i) == Some(&'*') {
            width = Some(take(&mut next)?.as_int()?);
            i += 1;
        } else {
            let start = i;
            while i < spec.len() && spec[i].is_ascii_digit() {
                i += 1;
            }
            if i > start {
                width = Some(
                    spec[start..i]
                        .iter()
                        .collect::<String>()
                        .parse::<MoltInt>()
                        .unwrap(),
                );
            }
        }
        let mut precision = None;
        if spec.get(i) == Some(&'.') {
            i += 1;
            if spec.get(i) == Some(&'*') {
                precision = Some(take(&mut next)?.as_int()?.max(0) as usize);
                i += 1;
            } else {
                let start = i;
                while i < spec.len() && spec[i].is_ascii_digit() {
                    i += 1;
                }
                precision = Some(
                    spec[start..i]
                        .iter()
                        .collect::<String>()
                        .parse()
                        .unwrap_or(0),
                );
            }
        }
        while i < spec.len() && "hlLqjzt".contains(spec[i]) {
            i += 1;
        }
        let Some(&conversion) = spec.get(i) else {
            return molt_err!("format string ended in middle of field specifier");
        };
        i += 1;
        let mut left = flags.contains('-');
        if let Some(w) = width {
            if w < 0 {
                left = true;
            }
        }
        let width = width.map(|w| w.unsigned_abs() as usize).unwrap_or(0);
        let value = take(&mut next)?;
        let sign = |negative: bool| {
            if negative {
                "-"
            } else if flags.contains('+') {
                "+"
            } else if flags.contains(' ') {
                " "
            } else {
                ""
            }
        };
        let (prefix, body): (String, String) = match conversion {
            's' => {
                let mut text = value.to_string();
                if let Some(p) = precision {
                    text = text.chars().take(p).collect();
                }
                (String::new(), text)
            }
            'c' => (
                String::new(),
                char::from_u32(value.as_int()? as u32)
                    .unwrap_or('?')
                    .to_string(),
            ),
            'd' | 'i' | 'u' => {
                let number = value.as_int()?;
                let mut digits = number.unsigned_abs().to_string();
                if let Some(p) = precision {
                    while digits.len() < p {
                        digits.insert(0, '0');
                    }
                }
                (sign(number < 0).to_string(), digits)
            }
            'o' | 'x' | 'X' | 'b' => {
                let number = value.as_int()? as u64;
                let mut digits = match conversion {
                    'o' => format!("{number:o}"),
                    'x' => format!("{number:x}"),
                    'X' => format!("{number:X}"),
                    _ => format!("{number:b}"),
                };
                if let Some(p) = precision {
                    while digits.len() < p {
                        digits.insert(0, '0');
                    }
                }
                let prefix = if flags.contains('#') && number != 0 {
                    match conversion {
                        'o' => "0",
                        'x' => "0x",
                        'X' => "0X",
                        _ => "0b",
                    }
                } else {
                    ""
                };
                (prefix.to_string(), digits)
            }
            'f' | 'F' | 'e' | 'E' | 'g' | 'G' => {
                let number = value.as_float()?;
                let p = precision.unwrap_or(6);
                let magnitude = number.abs();
                let body = match conversion {
                    'f' | 'F' => format!("{magnitude:.p$}"),
                    'e' | 'E' => exponent(magnitude, p, conversion == 'E'),
                    _ => general(magnitude, p, conversion == 'G', flags.contains('#')),
                };
                (
                    sign(number.is_sign_negative() && number != 0.0).to_string(),
                    body,
                )
            }
            other => return molt_err!("bad field specifier \"{}\"", other),
        };
        let length = prefix.chars().count() + body.chars().count();
        if length >= width {
            output.push_str(&prefix);
            output.push_str(&body);
        } else if left {
            output.push_str(&prefix);
            output.push_str(&body);
            output.push_str(&" ".repeat(width - length));
        } else if flags.contains('0') && conversion != 's' && conversion != 'c' {
            output.push_str(&prefix);
            output.push_str(&"0".repeat(width - length));
            output.push_str(&body);
        } else {
            output.push_str(&" ".repeat(width - length));
            output.push_str(&prefix);
            output.push_str(&body);
        }
    }
    molt_ok!(output)
}

fn exponent(number: f64, precision: usize, upper: bool) -> String {
    let text = format!("{number:.precision$e}");
    let (mantissa, power) = text.split_once('e').unwrap();
    let power: i32 = power.parse().unwrap();
    let result = format!(
        "{mantissa}e{}{:02}",
        if power < 0 { '-' } else { '+' },
        power.abs()
    );
    if upper {
        result.to_uppercase()
    } else {
        result
    }
}

fn general(number: f64, precision: usize, upper: bool, alternate: bool) -> String {
    let precision = precision.max(1);
    if number == 0.0 {
        return "0".into();
    }
    let power = number.log10().floor() as i32;
    let mut text = if power < -4 || power >= precision as i32 {
        exponent(number, precision - 1, upper)
    } else {
        let decimals = (precision as i32 - 1 - power).max(0) as usize;
        format!("{number:.decimals$}")
    };
    if !alternate {
        if let Some(e) = text.find(['e', 'E']) {
            let (mantissa, rest) = text.split_at(e);
            let mantissa = if mantissa.contains('.') {
                mantissa.trim_end_matches('0').trim_end_matches('.')
            } else {
                mantissa
            };
            text = format!("{mantissa}{rest}");
        } else if text.contains('.') {
            text = text.trim_end_matches('0').trim_end_matches('.').to_string();
        }
    }
    text
}

// ---------------------------------------------------------------------------
// string subcommands

fn chars(value: &Value) -> Vec<char> {
    value.as_str().chars().collect()
}

fn char_index(value: &Value, length: usize) -> Result<MoltInt, Exception> {
    list_ops::index(value, length)
}

pub fn cmd_string_first(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(2, argv, 4, 5, "needleString haystackString ?startIndex?")?;
    let needle = chars(&argv[2]);
    let haystack = chars(&argv[3]);
    let start = match argv.get(4) {
        Some(index) => char_index(index, haystack.len())?.max(0) as usize,
        None => 0,
    };
    if needle.is_empty() {
        return molt_ok!(-1);
    }
    let found = (start..haystack.len())
        .find(|&i| haystack[i..].starts_with(&needle))
        .map(|i| i as MoltInt)
        .unwrap_or(-1);
    molt_ok!(found)
}

pub fn cmd_string_last(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(2, argv, 4, 5, "needleString haystackString ?lastIndex?")?;
    let needle = chars(&argv[2]);
    let haystack = chars(&argv[3]);
    let last = match argv.get(4) {
        Some(index) => char_index(index, haystack.len())?,
        None => haystack.len() as MoltInt,
    };
    if needle.is_empty() || last < 0 {
        return molt_ok!(-1);
    }
    let last = (last as usize).min(haystack.len());
    let found = (0..=last)
        .rev()
        .find(|&i| i + needle.len() <= haystack.len() && haystack[i..].starts_with(&needle))
        .map(|i| i as MoltInt)
        .unwrap_or(-1);
    molt_ok!(found)
}

pub fn cmd_string_index(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(2, argv, 4, 4, "string charIndex")?;
    let text = chars(&argv[2]);
    let index = char_index(&argv[3], text.len())?;
    molt_ok!(usize::try_from(index)
        .ok()
        .and_then(|i| text.get(i))
        .map(|c| c.to_string())
        .unwrap_or_default())
}

fn range(text: &[char], first: &Value, last: &Value) -> Result<(usize, usize), Exception> {
    let length = text.len();
    let first = char_index(first, length)?.max(0) as usize;
    let last = char_index(last, length)?;
    if last < 0 {
        return Ok((0, 0));
    }
    let last = (last as usize).min(length.saturating_sub(1));
    if first > last || first >= length {
        return Ok((0, 0));
    }
    Ok((first, last + 1))
}

pub fn cmd_string_range(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(2, argv, 5, 5, "string first last")?;
    let text = chars(&argv[2]);
    let (start, end) = range(&text, &argv[3], &argv[4])?;
    molt_ok!(text[start..end].iter().collect::<String>())
}

pub fn cmd_string_replace(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(2, argv, 5, 6, "string first last ?string?")?;
    let text = chars(&argv[2]);
    let (start, end) = range(&text, &argv[3], &argv[4])?;
    if start == end {
        return molt_ok!(argv[2].clone());
    }
    let mut result: String = text[..start].iter().collect();
    if let Some(replacement) = argv.get(5) {
        result.push_str(replacement.as_str());
    }
    result.extend(&text[end..]);
    molt_ok!(result)
}

fn case_map(argv: &[Value], map: impl Fn(&str) -> String) -> MoltResult {
    check_args(2, argv, 3, 5, "string ?first? ?last?")?;
    let text = chars(&argv[2]);
    if argv.len() == 3 {
        return molt_ok!(map(argv[2].as_str()));
    }
    let last = argv.get(4).cloned().unwrap_or_else(|| argv[3].clone());
    let (start, end) = range(&text, &argv[3], &last)?;
    let mut result: String = text[..start].iter().collect();
    result.push_str(&map(&text[start..end].iter().collect::<String>()));
    result.extend(&text[end..]);
    molt_ok!(result)
}

pub fn cmd_string_tolower(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    case_map(argv, str::to_lowercase)
}

pub fn cmd_string_toupper(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    case_map(argv, str::to_uppercase)
}

pub fn cmd_string_totitle(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    case_map(argv, |text| {
        let mut characters = text.chars();
        match characters.next() {
            Some(first) => first
                .to_uppercase()
                .chain(characters.flat_map(char::to_lowercase))
                .collect(),
            None => String::new(),
        }
    })
}

fn trim(argv: &[Value], left: bool, right: bool) -> MoltResult {
    check_args(2, argv, 3, 4, "string ?chars?")?;
    let set: Vec<char> = match argv.get(3) {
        Some(characters) => characters.as_str().chars().collect(),
        None => vec![' ', '\t', '\n', '\r', '\x0b', '\x0c', '\0'],
    };
    let mut text = argv[2].as_str();
    if left {
        text = text.trim_start_matches(|c| set.contains(&c));
    }
    if right {
        text = text.trim_end_matches(|c| set.contains(&c));
    }
    molt_ok!(text)
}

pub fn cmd_string_trim(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    trim(argv, true, true)
}

pub fn cmd_string_trimleft(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    trim(argv, true, false)
}

pub fn cmd_string_trimright(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    trim(argv, false, true)
}

pub fn cmd_string_repeat(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(2, argv, 4, 4, "string count")?;
    let count = argv[3].as_int()?.max(0) as usize;
    molt_ok!(argv[2].as_str().repeat(count))
}

pub fn cmd_string_reverse(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(2, argv, 3, 3, "string")?;
    molt_ok!(argv[2].as_str().chars().rev().collect::<String>())
}

pub fn cmd_string_is(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(2, argv, 4, 7, "class ?-strict? ?-failindex var? string")?;
    let class = argv[2].as_str();
    let mut strict = false;
    let mut fail_var = None;
    let mut position = 3;
    while position < argv.len() - 1 {
        match argv[position].as_str() {
            "-strict" => strict = true,
            "-failindex" => {
                position += 1;
                fail_var = argv.get(position).cloned();
            }
            other => return molt_err!("bad option \"{}\": must be -strict or -failindex", other),
        }
        position += 1;
    }
    const CLASSES: [&str; 22] = [
        "alnum", "alpha", "ascii", "boolean", "control", "digit", "double", "entier", "false",
        "graph", "integer", "list", "lower", "print", "punct", "space", "true", "upper", "wide",
        "wordchar", "xdigit", "dict",
    ];
    // Tcl accepts any unique abbreviation of a class name.
    let class = match CLASSES
        .iter()
        .filter(|name| name.starts_with(class))
        .collect::<Vec<_>>()
        .as_slice()
    {
        [only] => **only,
        _ if CLASSES.contains(&class) => class,
        _ => return molt_err!("bad class \"{}\"", class),
    };
    let value = &argv[argv.len() - 1];
    let text = value.as_str();
    if text.is_empty() {
        return molt_ok!(!strict);
    }
    let all = |predicate: fn(char) -> bool| text.chars().position(|c| !predicate(c));
    let failure = match class {
        "alnum" => all(char::is_alphanumeric),
        "alpha" => all(char::is_alphabetic),
        "ascii" => all(|c| c.is_ascii()),
        "control" => all(char::is_control),
        "digit" => all(|c| c.is_ascii_digit()),
        "graph" => all(|c| !c.is_whitespace() && !c.is_control()),
        "lower" => all(char::is_lowercase),
        "upper" => all(char::is_uppercase),
        "print" => all(|c| !c.is_control()),
        "punct" => all(|c| c.is_ascii_punctuation()),
        "space" => all(char::is_whitespace),
        "wordchar" => all(|c| c.is_alphanumeric() || c == '_'),
        "xdigit" => all(|c| c.is_ascii_hexdigit()),
        "integer" | "entier" | "wide" => (value.as_int().is_err()).then_some(0),
        "double" => (value.as_float().is_err()).then_some(0),
        "boolean" => (value.as_bool().is_err()).then_some(0),
        "true" => (!matches!(value.as_bool(), Ok(true))).then_some(0),
        "false" => (!matches!(value.as_bool(), Ok(false))).then_some(0),
        "list" | "dict" => (value.as_list().is_err()).then_some(0),
        other => return molt_err!("bad class \"{}\"", other),
    };
    if let (Some(index), Some(name)) = (failure, fail_var) {
        interp.set_var(&name, Value::from(index as MoltInt))?;
    }
    molt_ok!(failure.is_none())
}

// ---------------------------------------------------------------------------
// glob

fn expand_braces(pattern: &str) -> Vec<String> {
    let Some(open) = pattern.find('{') else {
        return vec![pattern.to_string()];
    };
    let mut depth = 0;
    let mut close = None;
    for (offset, character) in pattern[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(close) = close else {
        return vec![pattern.to_string()];
    };
    let (head, body, tail) = (
        &pattern[..open],
        &pattern[open + 1..close],
        &pattern[close + 1..],
    );
    let mut alternatives = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (offset, character) in body.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                alternatives.push(&body[start..offset]);
                start = offset + 1;
            }
            _ => {}
        }
    }
    alternatives.push(&body[start..]);
    alternatives
        .into_iter()
        .flat_map(|alternative| expand_braces(&format!("{head}{alternative}{tail}")))
        .collect()
}

fn glob_one(base: &str, pattern: &str) -> Vec<String> {
    let absolute = pattern.starts_with('/');
    let mut paths = vec![if absolute {
        "/".to_string()
    } else {
        String::new()
    }];
    for segment in pattern.split('/').filter(|s| !s.is_empty()) {
        let mut next = Vec::new();
        let wild = segment.contains(['*', '?', '[', '\\']);
        for path in &paths {
            let directory = if path.is_empty() {
                if base.is_empty() {
                    ".".to_string()
                } else {
                    base.to_string()
                }
            } else if absolute || base.is_empty() {
                path.clone()
            } else {
                format!("{base}/{path}")
            };
            let join = |name: &str| {
                if path.is_empty() {
                    name.to_string()
                } else if path.ends_with('/') {
                    format!("{path}{name}")
                } else {
                    format!("{path}/{name}")
                }
            };
            if !wild {
                let candidate = join(segment);
                let full = if absolute || base.is_empty() {
                    candidate.clone()
                } else {
                    format!("{base}/{candidate}")
                };
                if std::fs::symlink_metadata(&full).is_ok() {
                    next.push(candidate);
                }
                continue;
            }
            let matcher = Glob::new(segment, true);
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            let mut names: Vec<String> = entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| {
                    (!name.starts_with('.') || segment.starts_with('.')) && matcher.matches(name)
                })
                .collect();
            names.sort();
            next.extend(names.iter().map(|name| join(name)));
        }
        paths = next;
    }
    paths.retain(|path| !path.is_empty());
    paths
}

/// # glob ?switches? pattern ?pattern ...?
pub fn cmd_glob(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 0, "?switches? name ?name ...?")?;
    let (mut nocomplain, mut tails, mut join) = (false, false, false);
    let mut directory = String::new();
    let mut prefix = String::new();
    let mut types: Vec<String> = Vec::new();
    let mut position = 1;
    const GLOB_OPTIONS: &[&str] = &[
        "-nocomplain",
        "-tails",
        "-join",
        "-directory",
        "-path",
        "-types",
        "--",
    ];
    while position < argv.len() && argv[position].as_str().starts_with('-') {
        match option_prefix(argv[position].as_str(), GLOB_OPTIONS) {
            "-nocomplain" => nocomplain = true,
            "-tails" => tails = true,
            "-join" => join = true,
            "-directory" => {
                position += 1;
                directory = argv.get(position).map(Value::to_string).unwrap_or_default();
            }
            "-path" => {
                position += 1;
                prefix = argv.get(position).map(Value::to_string).unwrap_or_default();
            }
            "-types" => {
                position += 1;
                if let Some(value) = argv.get(position) {
                    types = value.as_list()?.iter().map(Value::to_string).collect();
                }
            }
            "--" => {
                position += 1;
                break;
            }
            other => return molt_err!("bad option \"{}\"", other),
        }
        position += 1;
    }
    let patterns: Vec<String> = if join {
        vec![argv[position..]
            .iter()
            .map(Value::as_str)
            .collect::<Vec<_>>()
            .join("/")]
    } else {
        argv[position..].iter().map(Value::to_string).collect()
    };
    let mut results = Vec::new();
    for pattern in patterns {
        for expanded in expand_braces(&format!("{prefix}{pattern}")) {
            for path in glob_one(&directory, &expanded) {
                let full = if directory.is_empty() || path.starts_with('/') {
                    path.clone()
                } else {
                    format!("{}/{path}", directory.trim_end_matches('/'))
                };
                if !types.is_empty() {
                    let Ok(metadata) = std::fs::symlink_metadata(&full) else {
                        continue;
                    };
                    let file_type = metadata.file_type();
                    let followed = std::fs::metadata(&full).ok();
                    let ok = types.iter().any(|kind| match kind.as_str() {
                        "d" => followed.as_ref().is_some_and(|m| m.is_dir()),
                        "f" => followed.as_ref().is_some_and(|m| m.is_file()),
                        "l" => file_type.is_symlink(),
                        "p" | "s" | "b" | "c" => false,
                        _ => true,
                    });
                    if !ok {
                        continue;
                    }
                }
                results.push(Value::from(if tails || directory.is_empty() {
                    path
                } else {
                    full
                }));
            }
        }
    }
    if results.is_empty() && !nocomplain {
        let names = argv[position..]
            .iter()
            .map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" ");
        return molt_err!("no files matched glob pattern \"{}\"", names);
    }
    molt_ok!(Value::from(results))
}

/// The option `given` names: an exact match or, as Tcl allows, a unique
/// prefix. Anything else is returned unchanged for the caller to reject.
fn option_prefix<'a>(given: &'a str, options: &[&'a str]) -> &'a str {
    if options.contains(&given) || given.len() < 2 {
        return given;
    }
    let mut matches = options.iter().filter(|option| option.starts_with(given));
    match (matches.next(), matches.next()) {
        (Some(option), None) => option,
        _ => given,
    }
}

/// # clock seconds | clock milliseconds
pub fn cmd_clock(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 0, "subcommand ?arg ...?")?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    match argv[1].as_str() {
        "seconds" => molt_ok!(now.as_secs() as MoltInt),
        "milliseconds" => molt_ok!(now.as_millis() as MoltInt),
        "microseconds" => molt_ok!(now.as_micros() as MoltInt),
        "format" => clock_format(argv),
        "scan" => clock_scan(argv),
        other => molt_err!("unsupported clock subcommand \"{}\"", other),
    }
}

/// `-format`, and whether `-gmt`/`-timezone` select UTC.
fn clock_options(argv: &[Value], first: usize) -> Result<(Option<String>, bool), Exception> {
    let mut format = None;
    let mut gmt = false;
    let mut index = first;
    while index < argv.len() {
        let value = argv.get(index + 1).ok_or_else(|| {
            Exception::molt_err(Value::from(format!(
                "missing value for option \"{}\"",
                argv[index]
            )))
        })?;
        match argv[index].as_str() {
            "-format" => format = Some(value.as_str().to_string()),
            "-gmt" => gmt = value.as_bool()?,
            "-timezone" => {
                gmt = matches!(value.as_str(), ":UTC" | "UTC" | ":GMT" | "GMT" | "+0000")
            }
            "-locale" | "-base" => {}
            other => return molt_err!("bad option \"{}\"", other),
        }
        index += 2;
    }
    Ok((format, gmt))
}

/// # clock format seconds ?-format format? ?-gmt boolean? ?-timezone zone?
fn clock_format(argv: &[Value]) -> MoltResult {
    check_args(2, argv, 3, 0, "clockval ?-option value ...?")?;
    let seconds = argv[2].as_int()? as libc::time_t;
    let (format, gmt) = clock_options(argv, 3)?;
    let format = format.unwrap_or_else(|| "%a %b %d %H:%M:%S %Z %Y".into());
    let format =
        std::ffi::CString::new(format).map_err(|_| Exception::molt_err("invalid format".into()))?;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers refer to live, initialized values.
    unsafe {
        if gmt {
            libc::gmtime_r(&seconds, &mut tm);
        } else {
            libc::localtime_r(&seconds, &mut tm);
        }
    }
    let mut buffer = vec![0u8; 1024];
    // SAFETY: the buffer length bounds the write; format is NUL terminated.
    let length = unsafe {
        libc::strftime(
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            format.as_ptr(),
            &tm,
        )
    };
    buffer.truncate(length);
    molt_ok!(String::from_utf8_lossy(&buffer).into_owned())
}

/// # clock scan string ?-format format? ?-gmt boolean? ?-timezone zone?
///
/// Without `-format`, ISO 8601 dates and times are recognized.
fn clock_scan(argv: &[Value]) -> MoltResult {
    check_args(2, argv, 3, 0, "string ?-option value ...?")?;
    let (format, gmt) = clock_options(argv, 3)?;
    let text = argv[2].as_str().trim();
    let formats: Vec<String> = match format {
        Some(format) => vec![format],
        None => [
            "%Y-%m-%dT%H:%M:%SZ",
            "%Y-%m-%dT%H:%M:%S",
            "%Y-%m-%d %H:%M:%S",
            "%Y-%m-%d %H:%M",
            "%Y-%m-%d",
            "%Y%m%dT%H%M%S",
            "%Y%m%d",
        ]
        .iter()
        .map(|format| format.to_string())
        .collect(),
    };
    let input =
        std::ffi::CString::new(text).map_err(|_| Exception::molt_err("invalid date".into()))?;
    for format in formats {
        let pattern = match std::ffi::CString::new(format) {
            Ok(pattern) => pattern,
            Err(_) => continue,
        };
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        // SAFETY: both strings are NUL terminated and tm is a valid output.
        let end = unsafe { libc::strptime(input.as_ptr(), pattern.as_ptr(), &mut tm) };
        // SAFETY: strptime returns a pointer into `input` on success.
        if end.is_null() || unsafe { *end } != 0 {
            continue;
        }
        tm.tm_isdst = -1;
        // SAFETY: tm was filled by strptime.
        let seconds = unsafe {
            if gmt {
                libc::timegm(&mut tm)
            } else {
                libc::mktime(&mut tm)
            }
        };
        return molt_ok!(seconds as MoltInt);
    }
    molt_err!("unable to convert date-time string \"{}\"", text)
}

// ---------------------------------------------------------------------------
// scan

/// # scan string format ?varName ...?
pub fn cmd_scan(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 0, "string format ?varName ...?")?;
    let input: Vec<char> = argv[1].as_str().chars().collect();
    let format: Vec<char> = argv[2].as_str().chars().collect();
    let mut values: Vec<Value> = Vec::new();
    let (mut i, mut f) = (0, 0);
    let mut failed_early = false;
    while f < format.len() {
        let c = format[f];
        if c.is_whitespace() {
            while i < input.len() && input[i].is_whitespace() {
                i += 1;
            }
            f += 1;
            continue;
        }
        if c != '%' {
            if i < input.len() && input[i] == c {
                i += 1;
                f += 1;
                continue;
            }
            break;
        }
        f += 1;
        if format.get(f) == Some(&'%') {
            if input.get(i) == Some(&'%') {
                i += 1;
                f += 1;
                continue;
            }
            break;
        }
        let suppress = format.get(f) == Some(&'*');
        if suppress {
            f += 1;
        }
        let start = f;
        while f < format.len() && format[f].is_ascii_digit() {
            f += 1;
        }
        let width: Option<usize> = format[start..f].iter().collect::<String>().parse().ok();
        while f < format.len() && "hlLjzt".contains(format[f]) {
            f += 1;
        }
        let Some(&conversion) = format.get(f) else {
            return molt_err!("unterminated format specifier");
        };
        f += 1;
        let limit = |from: usize| {
            width
                .map(|w| (from + w).min(input.len()))
                .unwrap_or(input.len())
        };
        if !matches!(conversion, 'c' | '[' | 'n') {
            while i < input.len() && input[i].is_whitespace() {
                i += 1;
            }
        }
        if i >= input.len() && conversion != 'n' {
            failed_early = values.is_empty();
            break;
        }
        let value = match conversion {
            'd' | 'u' | 'i' | 'o' | 'x' | 'X' | 'b' => {
                let end = limit(i);
                let mut j = i;
                if j < end && (input[j] == '-' || input[j] == '+') {
                    j += 1;
                }
                let radix = match conversion {
                    'o' => 8,
                    'x' | 'X' => 16,
                    'b' => 2,
                    _ => 10,
                };
                let digits_start = j;
                while j < end && input[j].is_digit(radix) {
                    j += 1;
                }
                if j == digits_start {
                    break;
                }
                let text: String = input[i..j].iter().collect();
                i = j;
                let number = MoltInt::from_str_radix(&text, radix).map_err(|_| {
                    Exception::molt_err(Value::from("integer value too large to represent"))
                })?;
                Value::from(number)
            }
            'f' | 'e' | 'E' | 'g' | 'G' => {
                let end = limit(i);
                let mut j = i;
                if j < end && (input[j] == '-' || input[j] == '+') {
                    j += 1;
                }
                while j < end && (input[j].is_ascii_digit() || input[j] == '.') {
                    j += 1;
                }
                if j < end && (input[j] == 'e' || input[j] == 'E') {
                    let mark = j;
                    j += 1;
                    if j < end && (input[j] == '-' || input[j] == '+') {
                        j += 1;
                    }
                    if j < end && input[j].is_ascii_digit() {
                        while j < end && input[j].is_ascii_digit() {
                            j += 1;
                        }
                    } else {
                        j = mark;
                    }
                }
                let text: String = input[i..j].iter().collect();
                let Ok(number) = text.parse::<f64>() else {
                    break;
                };
                i = j;
                Value::from(number)
            }
            's' => {
                let end = limit(i);
                let mut j = i;
                while j < end && !input[j].is_whitespace() {
                    j += 1;
                }
                let text: String = input[i..j].iter().collect();
                i = j;
                Value::from(text)
            }
            'c' => {
                let character = input[i];
                i += 1;
                Value::from(character as MoltInt)
            }
            'n' => Value::from(i as MoltInt),
            '[' => {
                let mut set = Vec::new();
                let negate = format.get(f) == Some(&'^');
                if negate {
                    f += 1;
                }
                if format.get(f) == Some(&']') {
                    set.push(']');
                    f += 1;
                }
                while f < format.len() && format[f] != ']' {
                    if format.get(f + 1) == Some(&'-')
                        && f + 2 < format.len()
                        && format[f + 2] != ']'
                    {
                        for character in format[f]..=format[f + 2] {
                            set.push(character);
                        }
                        f += 3;
                    } else {
                        set.push(format[f]);
                        f += 1;
                    }
                }
                f += 1;
                let end = limit(i);
                let mut j = i;
                while j < end && (set.contains(&input[j]) != negate) {
                    j += 1;
                }
                if j == i {
                    break;
                }
                let text: String = input[i..j].iter().collect();
                i = j;
                Value::from(text)
            }
            other => return molt_err!("bad scan conversion character \"{}\"", other),
        };
        if !suppress {
            values.push(value);
        }
    }
    let variables = &argv[3..];
    if variables.is_empty() {
        if failed_early {
            return molt_ok!(-1);
        }
        return molt_ok!(Value::from(values));
    }
    if failed_early {
        return molt_ok!(-1);
    }
    for (name, value) in variables.iter().zip(values.iter()) {
        interp.set_var(name, value.clone())?;
    }
    molt_ok!(values.len().min(variables.len()) as MoltInt)
}
