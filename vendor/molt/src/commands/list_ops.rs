//! List slicing, end-relative indices, and delimiter splitting.
use crate::{check_args, molt_ok, ContextID, Exception, Interp, MoltInt, MoltResult, Value};

pub(super) fn index(value: &Value, length: usize) -> Result<MoltInt, Exception> {
    if let Ok(number) = value.as_int() {
        return Ok(number);
    }
    let text = value.as_str().trim();
    let end = length as MoltInt - 1;
    if text == "end" {
        return Ok(end);
    }
    let parse = || -> Option<MoltInt> {
        let (base, rest) = if let Some(rest) = text.strip_prefix("end") {
            (end, rest)
        } else {
            let position = text
                .char_indices()
                .skip(1)
                .find(|(_, c)| matches!(c, '+' | '-'))?
                .0;
            (
                Value::from(&text[..position]).as_int().ok()?,
                &text[position..],
            )
        };
        let operation = rest.as_bytes().first()?;
        if !matches!(operation, b'+' | b'-') {
            return None;
        }
        let offset = Value::from(&rest[1..]).as_int().ok()?;
        Some(if *operation == b'+' {
            base.saturating_add(offset)
        } else {
            base.saturating_sub(offset)
        })
    };
    parse().ok_or_else(|| {
        Exception::molt_err(Value::from(format!(
            "bad index \"{}\": must be integer?[+-]integer? or end?[+-]integer?",
            value
        )))
    })
}

pub fn cmd_lrange(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 4, 4, "list first last")?;
    let list = argv[1].as_list()?;
    let first = index(&argv[2], list.len())?.max(0);
    let last = index(&argv[3], list.len())?.min(list.len() as MoltInt - 1);
    if first > last {
        return Ok(Value::empty());
    }
    molt_ok!(&list[first as usize..=last as usize])
}

pub fn cmd_split(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 3, "string ?splitChars?")?;
    let text = argv[1].as_str();
    let separators = argv.get(2).map(Value::as_str).unwrap_or(" \n\t\r");
    let values: Vec<Value> = if text.is_empty() {
        Vec::new()
    } else if separators.is_empty() {
        text.chars().map(|c| Value::from(c.to_string())).collect()
    } else {
        text.split(|c| separators.contains(c))
            .map(Value::from)
            .collect()
    };
    molt_ok!(values)
}
