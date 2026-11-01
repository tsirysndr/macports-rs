//! TCL List Parsing and Formatting

use crate::molt_err;
use crate::tokenizer::Tokenizer;
use crate::types::*;
use crate::value::Value;

//--------------------------------------------------------------------------
// List Parsing

/// Parses a list-formatted string into a vector, throwing
/// a Molt error if the list cannot be parsed as a list.
pub(crate) fn get_list(str: &str) -> Result<MoltList, Exception> {
    let mut ctx = Tokenizer::new(str);

    parse_list(&mut ctx)
}

// Is the character a valid whitespace character in list syntax?
fn is_list_white(ch: char) -> bool {
    match ch {
        ' ' => true,
        '\n' => true,
        '\r' => true,
        '\t' => true,
        '\x0B' => true, // Vertical Tab
        '\x0C' => true, // Form Feed
        _ => false,
    }
}

fn parse_list(ctx: &mut Tokenizer) -> Result<MoltList, Exception> {
    // FIRST, skip any list whitespace.
    ctx.skip_while(|ch| is_list_white(*ch));

    // Read words until we get to the end of the input or hit an error
    let mut items = Vec::new();
    while !ctx.at_end() {
        // FIRST, get the next item; there has to be one.
        // Throw an error if there's a formatting problem.
        items.push(parse_item(ctx)?);

        // NEXT, skip whitespace to the end or the next item.
        ctx.skip_while(|ch| is_list_white(*ch));
    }

    // NEXT, return the items.
    Ok(items)
}

/// We're at the beginning of an item in the list.
/// It's either a bare word, a braced string, or a quoted string--or there's
/// an error in the input.  Whichever it is, get it.
fn parse_item(ctx: &mut Tokenizer) -> MoltResult {
    if ctx.is('{') {
        Ok(parse_braced_item(ctx)?)
    } else if ctx.is('"') {
        Ok(parse_quoted_item(ctx)?)
    } else {
        Ok(parse_bare_item(ctx)?)
    }
}

/// Parse a braced item.  We need to count braces, so that they balance; and
/// we need to handle backslashes in the input, so that quoted braces don't count.
fn parse_braced_item(ctx: &mut Tokenizer) -> MoltResult {
    // FIRST, we have to count braces.  Skip the first one, and count it.
    // Also, mark the following character, as we'll be accumulating a
    // token.
    ctx.next();
    let mut count = 1;

    // NEXT, mark the start of the token, and skip characters until we find the end.
    let mark = ctx.mark();
    while let Some(c) = ctx.peek() {
        if c == '\\' {
            // Backslash handling. Retain backslashes as is.
            // Note: this means that escaped '{' and '}' characters
            // don't affect the count.
            ctx.skip();
            ctx.skip();
        } else if c == '{' {
            count += 1;
            ctx.skip();
        } else if c == '}' {
            count -= 1;

            if count > 0 {
                ctx.skip();
            } else {
                // We've found and consumed the closing brace.  We should either
                // see more more whitespace, or we should be at the end of the list
                // Otherwise, there are incorrect characters following the close-brace.
                let result = Ok(Value::from(ctx.token(mark)));
                ctx.skip(); // Skip the closing brace

                if ctx.at_end() || ctx.has(|ch| is_list_white(*ch)) {
                    return result;
                } else {
                    return molt_err!("extra characters after close-brace");
                }
            }
        } else {
            ctx.skip();
        }
    }

    assert!(count > 0);
    molt_err!("unmatched open brace in list")
}

/// Parse a quoted item.  Does backslash substitution.
fn parse_quoted_item(ctx: &mut Tokenizer) -> MoltResult {
    // FIRST, consume the the opening quote.
    ctx.skip();

    let mut item = String::new();
    let mut start = ctx.mark();

    while !ctx.at_end() {
        ctx.skip_while(|ch| *ch != '"' && *ch != '\\');
        item.push_str(ctx.token(start));

        match ctx.peek() {
            Some('"') => {
                ctx.skip();
                return Ok(Value::from(item));
            }
            Some('\\') => {
                item.push(ctx.backslash_subst());
                start = ctx.mark();
            }
            _ => unreachable!(),
        }
    }

    molt_err!("unmatched open quote in list")
}

/// Parse a bare item.
fn parse_bare_item(ctx: &mut Tokenizer) -> MoltResult {
    let mut item = String::new();
    let mut start = ctx.mark();

    while !ctx.at_end() {
        // Note: the while condition ensures that there's a character.
        ctx.skip_while(|ch| !is_list_white(*ch) && *ch != '\\');

        item.push_str(ctx.token(start));
        start = ctx.mark();

        if ctx.has(|ch| is_list_white(*ch)) {
            break;
        }

        if ctx.is('\\') {
            item.push(ctx.backslash_subst());
            start = ctx.mark();
        }
    }

    Ok(Value::from(item))
}

//--------------------------------------------------------------------------
// List Formatting

/// Converts a list, represented as a vector of `Value`s, into a string, doing
/// all necessary quoting and escaping.
pub fn list_to_string(list: &[Value]) -> String {
    let mut vec: Vec<String> = Vec::new();

    let mut hash = !list.is_empty() && list[0].as_str().starts_with('#');

    for item in list {
        let item = item.to_string();
        match get_mode(&item) {
            Mode::AsIs => {
                if hash {
                    vec.push(brace_item(&item));
                } else {
                    vec.push(item)
                }
            }
            Mode::Brace => vec.push(brace_item(&item)),
            Mode::Escape => vec.push(escape_item(hash, &item, true)),
            // A leading "#" is braced rather than escaped, as Tcl 8.6 does.
            Mode::Mask if hash => vec.push(brace_item(&item)),
            Mode::Mask => vec.push(escape_item(hash, &item, false)),
        }
        hash = false;
    }

    vec.join(" ")
}

fn brace_item(item: &str) -> String {
    let mut word = String::new();
    word.push('{');
    word.push_str(item);
    word.push('}');
    word
}

/// Tcl's `TclConvertElement` escape forms: `Escape` also escapes braces,
/// `Mask` leaves them as they are.
fn escape_item(hash: bool, item: &str, braces: bool) -> String {
    let mut word = String::new();

    // If hash, the first character is a "#" that must be escaped.
    if hash {
        word.push('\\');
    }

    for ch in item.chars() {
        let control = match ch {
            '\n' => Some('n'),
            '\t' => Some('t'),
            '\r' => Some('r'),
            '\x0c' => Some('f'),
            '\x0b' => Some('v'),
            _ => None,
        };
        if let Some(letter) = control {
            word.push('\\');
            word.push(letter);
            continue;
        }
        match ch {
            ']' | '[' | '$' | ';' | ' ' | '\\' | '"' => word.push('\\'),
            '{' | '}' if braces => word.push('\\'),
            _ => {}
        }
        word.push(ch);
    }

    word
}

#[derive(Eq, PartialEq, Debug)]
enum Mode {
    AsIs,
    Brace,
    Escape,
    Mask,
}

/// Tcl 8.6's `TclScanElement` (with its compatibility rules): how an element
/// must be quoted to read back as itself.
fn get_mode(word: &str) -> Mode {
    if word.is_empty() {
        return Mode::Brace;
    }
    let bytes = word.as_bytes();
    let mut forbid_none = false;
    let mut require_escape = false;
    let mut prefer_escape = false;
    let mut prefer_brace = false;
    let mut nesting = 0i64;
    if bytes[0] == b'{' || bytes[0] == b'"' {
        forbid_none = true;
        prefer_brace = true;
    }
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => nesting += 1,
            b'}' => {
                nesting -= 1;
                if nesting < 0 {
                    require_escape = true;
                }
            }
            b']' | b'"' => {
                forbid_none = true;
                prefer_escape = true;
            }
            b'[' | b'$' | b';' => {
                forbid_none = true;
                prefer_brace = true;
            }
            b'\\' => {
                match bytes.get(i + 1) {
                    None => require_escape = true,
                    Some(b'\n') => {
                        require_escape = true;
                        i += 1;
                    }
                    Some(b'{' | b'}' | b'\\') => i += 1,
                    Some(_) => {}
                }
                forbid_none = true;
                prefer_brace = true;
            }
            b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r' => {
                forbid_none = true;
                prefer_brace = true;
            }
            _ => {}
        }
        i += 1;
    }
    if nesting != 0 {
        require_escape = true;
    }
    if require_escape {
        Mode::Escape
    } else if prefer_escape && !prefer_brace {
        Mode::Mask
    } else if forbid_none {
        Mode::Brace
    } else {
        Mode::AsIs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaped_elements_round_trip_control_characters_and_continuations() {
        for item in ["a\\\nb", "x\\\n", "{\n\t}\"", "\\\n\r\x0b\x0c"] {
            let list = list_to_string(&[Value::from(item), Value::from("z")]);
            let parsed = get_list(&list).unwrap();
            assert_eq!(parsed[0].as_str(), item, "{list:?}");
            assert_eq!(parsed[1].as_str(), "z");
        }
    }

    #[test]
    fn test_list_to_string() {
        assert_eq!(list_to_string(&[Value::from("a")]), "a");
        assert_eq!(list_to_string(&[Value::from("a"), Value::from("b")]), "a b");
        assert_eq!(
            list_to_string(&[Value::from("a"), Value::from("b"), Value::from("c")]),
            "a b c"
        );
        assert_eq!(
            list_to_string(&[Value::from("a"), Value::from(" "), Value::from("c")]),
            "a { } c"
        );
        assert_eq!(
            list_to_string(&[Value::from("a"), Value::from(""), Value::from("c")]),
            "a {} c"
        );
        assert_eq!(list_to_string(&[Value::from("a;b")]), "{a;b}");
        assert_eq!(list_to_string(&[Value::from("a$b")]), "{a$b}");
        assert_eq!(list_to_string(&[Value::from("a[b")]), "{a[b}");
        assert_eq!(list_to_string(&[Value::from("a]b")]), "a\\]b");
        // Results of Tcl 8.6 `list` for the same elements.
        let tcl = |items: &[&str]| {
            list_to_string(
                &items
                    .iter()
                    .map(|item| Value::from(*item))
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(tcl(&["a]{b}", "x"]), "a\\]{b} x");
        assert_eq!(tcl(&["#a]", "x"]), "{#a]} x");
        assert_eq!(
            tcl(&["port\"", "\"Mac", "a\"b c", "a]b"]),
            "port\\\" {\"Mac} {a\"b c} a\\]b"
        );
        assert_eq!(tcl(&["a\\{b", "x\\"]), "{a\\{b} x\\\\");
        assert_eq!(list_to_string(&[Value::from("a\\nb")]), "{a\\nb}");
        assert_eq!(
            list_to_string(&[Value::from("{ "), Value::from("abc")]),
            r#"\{\  abc"#
        );
    }

    #[test]
    fn test_parse_braced_item() {
        assert_eq!(pbi("{}"), "|".to_string());
        assert_eq!(pbi("{abc}"), "abc|".to_string());
        assert_eq!(pbi("{abc}  "), "abc|  ".to_string());
        assert_eq!(pbi("{a{b}c}"), "a{b}c|".to_string());
        assert_eq!(pbi("{a{b}{c}}"), "a{b}{c}|".to_string());
        assert_eq!(pbi("{a{b}{c}d}"), "a{b}{c}d|".to_string());
        assert_eq!(pbi("{a{b}{c}d} efg"), "a{b}{c}d| efg".to_string());
        assert_eq!(pbi("{a\\{bc}"), "a\\{bc|".to_string());
    }

    fn pbi(input: &str) -> String {
        let mut ctx = Tokenizer::new(input);
        if let Ok(val) = parse_braced_item(&mut ctx) {
            format!("{}|{}", val.as_str(), ctx.as_str())
        } else {
            String::from("Err")
        }
    }

    #[test]
    fn test_parse_quoted_item() {
        assert_eq!(pqi("\"abc\""), "abc|".to_string());
        assert_eq!(pqi("\"abc\"  "), "abc|  ".to_string());
        assert_eq!(pqi("\"a\\x77-\""), "aw-|".to_string());
    }

    fn pqi(input: &str) -> String {
        let mut ctx = Tokenizer::new(input);
        if let Ok(val) = parse_quoted_item(&mut ctx) {
            format!("{}|{}", val.as_str(), ctx.as_str())
        } else {
            String::from("Err")
        }
    }

    #[test]
    fn test_parse_bare_item() {
        println!("test_parse_bare_item");
        assert_eq!(pbare("abc"), "abc|".to_string());
        assert_eq!(pbare("abc def"), "abc| def".to_string());
        assert_eq!(pbare("abc\ndef"), "abc|\ndef".to_string());
        assert_eq!(pbare("abc\rdef"), "abc|\rdef".to_string());
        assert_eq!(pbare("abc\tdef"), "abc|\tdef".to_string());
        assert_eq!(pbare("abc\x0Bdef"), "abc|\x0Bdef".to_string());
        assert_eq!(pbare("abc\x0Cdef"), "abc|\x0Cdef".to_string());
        assert_eq!(pbare("a\\x77-"), "aw-|".to_string());
        assert_eq!(pbare("a\\x77- def"), "aw-| def".to_string());
        assert_eq!(pbare("a\\x77"), "aw|".to_string());
        assert_eq!(pbare("a\\x77 "), "aw| ".to_string());
    }

    fn pbare(input: &str) -> String {
        let mut ctx = Tokenizer::new(input);
        if let Ok(val) = parse_bare_item(&mut ctx) {
            format!("{}|{}", val.as_str(), ctx.as_str())
        } else {
            String::from("Err")
        }
    }

    // Most list parsing is tested in the Molt test suite.

    #[test]
    fn test_issue_43() {
        let list = get_list("a ;b c").unwrap();

        // If the list breaks on the semi-colon, the bug still exists.
        assert_eq!(list.len(), 3);
    }
}
