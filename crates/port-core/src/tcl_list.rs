//! Parses Tcl's serialized list format as data. Never evaluates Tcl code.
use anyhow::{Result, bail, ensure};
use std::collections::BTreeMap;

pub fn dictionary(raw: &str) -> Result<BTreeMap<String, String>> {
    let values = parse(raw)?;
    anyhow::ensure!(
        values.len().is_multiple_of(2),
        "malformed indexed dictionary"
    );
    Ok(values
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[k, v]| (k.clone(), v.clone()))
        .collect())
}

pub fn parse(input: &str) -> Result<Vec<String>> {
    let mut chars = input.chars().peekable();
    let mut words = Vec::new();
    while chars.peek().is_some() {
        while chars.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            chars.next();
        }
        let Some(first) = chars.next() else { break };
        let mut word = String::new();
        if first == '{' {
            let mut depth = 1;
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        let next = chars
                            .next()
                            .ok_or_else(|| anyhow::anyhow!("trailing backslash"))?;
                        if next == '\n' {
                            while chars.peek().is_some_and(|c| *c == ' ' || *c == '\t') {
                                chars.next();
                            }
                            word.push(' ');
                        } else {
                            word.push(c);
                            word.push(next);
                        }
                    }
                    '{' => {
                        depth += 1;
                        word.push(c);
                    }
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        word.push(c);
                    }
                    _ => word.push(c),
                }
            }
            ensure!(depth == 0, "unclosed brace in list");
            ensure!(
                chars.peek().is_none_or(|c| c.is_ascii_whitespace()),
                "characters after closing brace"
            );
        } else {
            let quoted = first == '"';
            let mut closed = !quoted;
            if !quoted {
                if first == '\\' {
                    word.push(escape(&mut chars)?);
                } else {
                    word.push(first);
                }
            }
            while let Some(&c) = chars.peek() {
                if !quoted && c.is_ascii_whitespace() {
                    break;
                }
                chars.next();
                if quoted && c == '"' {
                    closed = true;
                    break;
                }
                if c == '\\' {
                    word.push(escape(&mut chars)?);
                } else {
                    word.push(c);
                }
            }
            ensure!(closed, "unclosed quote in list");
            ensure!(
                chars.peek().is_none_or(|c| c.is_ascii_whitespace()),
                "characters after closing quote"
            );
        }
        words.push(word);
    }
    Ok(words)
}

fn escape(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Result<char> {
    let Some(c) = chars.next() else {
        bail!("trailing backslash");
    };
    Ok(match c {
        'a' => '\x07',
        'b' => '\x08',
        'f' => '\x0c',
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        'v' => '\x0b',
        '\n' => {
            while chars.peek().is_some_and(|c| *c == ' ' || *c == '\t') {
                chars.next();
            }
            ' '
        }
        'x' | 'u' | 'U' | '0'..='7' => {
            let (radix, limit, mut digits) = match c {
                'x' => (16, 2, String::new()),
                'u' => (16, 4, String::new()),
                'U' => (16, 8, String::new()),
                _ => (8, if c <= '3' { 3 } else { 2 }, c.to_string()),
            };
            while digits.len() < limit && chars.peek().is_some_and(|c| c.is_digit(radix)) {
                digits.push(chars.next().unwrap());
            }
            if digits.is_empty() {
                c
            } else {
                char::from_u32(u32::from_str_radix(&digits, radix)?).unwrap_or('\u{fffd}')
            }
        }
        _ => c,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_and_escaped() {
        assert_eq!(parse(r#"name {hello {nested} world} quoted "two words" empty {} escaped a\ b literal {$x [exec evil]}"#).unwrap(),
            ["name", "hello {nested} world", "quoted", "two words", "empty", "", "escaped", "a b", "literal", "$x [exec evil]"]);
    }
    #[test]
    fn escapes() {
        assert_eq!(
            parse(r#"\x41 \u00e9 \101 {a\{b\}}"#).unwrap(),
            ["A", "é", "A", r"a\{b\}"]
        );
    }
    #[test]
    fn malformed() {
        for s in ["{no", "\"no", "{a}b", "\"a\"b", "a\\"] {
            assert!(parse(s).is_err(), "{s}");
        }
    }
    #[test]
    fn continuation() {
        assert_eq!(parse("{hello\\\n  world}").unwrap(), ["hello world"]);
    }
}
