//! `regexp` and `regsub`, translating Tcl advanced regular expressions to the
//! `regex` crate where their syntax differs.
use crate::{molt_err, molt_ok, ContextID, Exception, Interp, MoltInt, MoltResult, Value};

/// A compiled pattern. Lookaround and backreferences, which Tcl's advanced
/// regular expressions allow, fall back to a backtracking engine.
pub(super) enum Pattern {
    Linear(regex::Regex),
    Backtracking(fancy_regex::Regex),
}

/// Byte ranges of the whole match and each group.
type Groups = Vec<Option<(usize, usize)>>;

impl Pattern {
    fn captures_at(&self, text: &str, offset: usize) -> Option<Groups> {
        match self {
            Pattern::Linear(regex) => regex.captures_at(text, offset).map(|captures| {
                (0..captures.len())
                    .map(|index| captures.get(index).map(|m| (m.start(), m.end())))
                    .collect()
            }),
            Pattern::Backtracking(regex) => regex
                .captures_from_pos(text, offset)
                .ok()
                .flatten()
                .map(|captures| {
                    (0..captures.len())
                        .map(|index| captures.get(index).map(|m| (m.start(), m.end())))
                        .collect()
                }),
        }
    }
    pub(super) fn is_match(&self, text: &str) -> bool {
        self.captures_at(text, 0).is_some()
    }
}

/// Compile a Tcl pattern. Tcl's default matching lets `.` and negated
/// brackets match newlines and anchors `^`/`$` only at string ends.
pub(super) fn compile(
    pattern: &str,
    nocase: bool,
    expanded: bool,
    line: bool,
) -> Result<Pattern, Exception> {
    // ARE directors: `***=` makes the rest literal, `***:` plain ARE.
    if let Some(literal) = pattern.strip_prefix("***=") {
        return compile(&regex::escape(literal), nocase, expanded, line);
    }
    let pattern = pattern.strip_prefix("***:").unwrap_or(pattern);
    let mut translated = String::with_capacity(pattern.len() + 8);
    let mut characters = pattern.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\\' {
            match characters.next() {
                Some('m') | Some('M') | Some('y') => translated.push_str(r"\b"),
                Some('Y') => translated.push_str(r"\B"),
                Some(other) => {
                    translated.push('\\');
                    translated.push(other);
                }
                None => translated.push_str(r"\\"),
            }
        } else {
            translated.push(character);
        }
    }
    let mut flags = String::new();
    if nocase {
        flags.push('i');
    }
    if expanded {
        flags.push('x');
    }
    if line {
        flags.push('m');
    } else {
        flags.push('s');
    }
    let source = format!("(?{flags}){translated}");
    match regex::Regex::new(&source) {
        Ok(regex) => Ok(Pattern::Linear(regex)),
        Err(err) => fancy_regex::Regex::new(&source)
            .map(Pattern::Backtracking)
            .map_err(|_| {
                Exception::molt_err(Value::from(format!(
                    "couldn't compile regular expression pattern: {err}"
                )))
            }),
    }
}

struct Options {
    nocase: bool,
    expanded: bool,
    line: bool,
    all: bool,
    inline: bool,
    indices: bool,
    start: Option<Value>,
}

fn options(argv: &[Value], allowed_inline: bool) -> Result<(Options, usize), Exception> {
    let mut options = Options {
        nocase: false,
        expanded: false,
        line: false,
        all: false,
        inline: false,
        indices: false,
        start: None,
    };
    let mut position = 1;
    // The pattern and string (and regsub's substitution) are never switches.
    let required = if allowed_inline { 2 } else { 3 };
    while position < argv.len()
        && argv.len() - position > required
        && argv[position].as_str().starts_with('-')
    {
        match argv[position].as_str() {
            "-nocase" => options.nocase = true,
            "-expanded" => options.expanded = true,
            "-line" | "-lineanchor" | "-linestop" => options.line = true,
            "-all" => options.all = true,
            "-inline" if allowed_inline => options.inline = true,
            "-indices" if allowed_inline => options.indices = true,
            "-about" => return molt_err!("regexp -about is not supported"),
            "-start" => {
                position += 1;
                options.start = argv.get(position).cloned();
            }
            "--" => {
                position += 1;
                break;
            }
            other => return molt_err!("bad option \"{}\"", other),
        }
        position += 1;
    }
    Ok((options, position))
}

/// Convert a character index to a byte offset within `text`.
fn byte_offset(text: &str, characters: usize) -> usize {
    text.char_indices()
        .nth(characters)
        .map(|(i, _)| i)
        .unwrap_or(text.len())
}

fn char_count(text: &str, bytes: usize) -> MoltInt {
    text[..bytes].chars().count() as MoltInt
}

fn start_offset(text: &str, start: &Option<Value>) -> Result<usize, Exception> {
    Ok(match start {
        Some(value) => {
            let length = text.chars().count();
            let index = super::list_ops::index(value, length)?.max(0) as usize;
            byte_offset(text, index.min(length))
        }
        None => 0,
    })
}

/// # regexp ?switches? exp string ?matchVar? ?subMatchVar ...?
pub fn cmd_regexp(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    let (options, position) = options(argv, true)?;
    if argv.len() < position + 2 {
        return molt_err!(
            "wrong # args: should be \"regexp ?-option ...? exp string ?matchVar? ?subMatchVar ...?\""
        );
    }
    let regex = compile(
        argv[position].as_str(),
        options.nocase,
        options.expanded,
        options.line,
    )?;
    let text = argv[position + 1].as_str();
    let variables = &argv[position + 2..];
    if options.inline && !variables.is_empty() {
        return molt_err!("regexp match variables not allowed when using -inline");
    }
    let start = start_offset(text, &options.start)?;
    let group = |captures: &Groups, index: usize| -> Value {
        match captures.get(index).copied().flatten() {
            Some((start, end)) if options.indices => Value::from(vec![
                Value::from(char_count(text, start)),
                Value::from(char_count(text, end) - 1),
            ]),
            Some((start, end)) => Value::from(&text[start..end]),
            None if options.indices => Value::from(vec![Value::from(-1), Value::from(-1)]),
            None => Value::empty(),
        }
    };
    let mut matches = Vec::new();
    let mut offset = start;
    while offset <= text.len() {
        let Some(captures) = regex.captures_at(text, offset) else {
            break;
        };
        let (whole_start, whole_end) = captures[0].unwrap();
        matches.push(captures);
        if !options.all {
            break;
        }
        offset = if whole_end == whole_start {
            match text[whole_end..].chars().next() {
                Some(c) => whole_end + c.len_utf8(),
                None => break,
            }
        } else {
            whole_end
        };
    }
    if options.inline {
        let mut result = Vec::new();
        for captures in &matches {
            for index in 0..captures.len() {
                result.push(group(captures, index));
            }
        }
        return molt_ok!(Value::from(result));
    }
    // As in Tcl, a failed match leaves the match variables untouched.
    if let Some(captures) = matches.last() {
        for (index, name) in variables.iter().enumerate() {
            interp.set_var(name, group(captures, index))?;
        }
    }
    if options.all {
        molt_ok!(matches.len() as MoltInt)
    } else {
        molt_ok!(!matches.is_empty())
    }
}

fn substitute(spec: &str, text: &str, captures: &Groups) -> String {
    let get = |index: usize| {
        captures
            .get(index)
            .copied()
            .flatten()
            .map(|(start, end)| &text[start..end])
            .unwrap_or("")
    };
    let mut result = String::new();
    let mut characters = spec.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '&' => result.push_str(get(0)),
            '\\' => match characters.peek().copied() {
                Some(digit @ '0'..='9') => {
                    characters.next();
                    let index = digit.to_digit(10).unwrap() as usize;
                    result.push_str(get(index));
                }
                Some(escaped @ ('&' | '\\')) => {
                    characters.next();
                    result.push(escaped);
                }
                _ => result.push('\\'),
            },
            other => result.push(other),
        }
    }
    result
}

/// # regsub ?switches? exp string subSpec ?varName?
pub fn cmd_regsub(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    let (options, position) = options(argv, false)?;
    if argv.len() < position + 3 || argv.len() > position + 4 {
        return molt_err!(
            "wrong # args: should be \"regsub ?-option ...? exp string subSpec ?varName?\""
        );
    }
    let regex = compile(
        argv[position].as_str(),
        options.nocase,
        options.expanded,
        options.line,
    )?;
    let text = argv[position + 1].as_str();
    let spec = argv[position + 2].as_str();
    let start = start_offset(text, &options.start)?;
    let mut result = String::from(&text[..start]);
    let mut offset = start;
    let mut count: MoltInt = 0;
    while offset <= text.len() {
        let Some(captures) = regex.captures_at(text, offset) else {
            break;
        };
        let (whole_start, whole_end) = captures[0].unwrap();
        result.push_str(&text[offset..whole_start]);
        result.push_str(&substitute(spec, text, &captures));
        count += 1;
        if whole_end == whole_start {
            match text[whole_end..].chars().next() {
                Some(c) => {
                    result.push(c);
                    offset = whole_end + c.len_utf8();
                }
                None => {
                    offset = text.len() + 1;
                    break;
                }
            }
        } else {
            offset = whole_end;
        }
        if !options.all {
            break;
        }
    }
    if offset <= text.len() {
        result.push_str(&text[offset..]);
    }
    match argv.get(position + 3) {
        Some(name) => {
            interp.set_var(name, Value::from(result))?;
            molt_ok!(count)
        }
        None => molt_ok!(result),
    }
}
