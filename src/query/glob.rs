//! Tcl-style string patterns, independent of filesystem path glob semantics.
#[derive(Debug)]
enum Token {
    Star,
    Any,
    Literal(char),
    Class(Vec<(char, char)>),
    Never,
}

#[derive(Debug)]
pub struct Glob {
    tokens: Vec<Token>,
    case_sensitive: bool,
}
impl Glob {
    pub fn new(pattern: &str, case_sensitive: bool) -> Self {
        let fold = |c: char| fold(c, case_sensitive);
        let mut chars = pattern.chars().peekable();
        let mut tokens = Vec::new();
        while let Some(c) = chars.next() {
            tokens.push(match c {
                '*' => {
                    if matches!(tokens.last(), Some(Token::Star)) {
                        continue;
                    }
                    Token::Star
                }
                '?' => Token::Any,
                '\\' => chars
                    .next()
                    .map(|c| Token::Literal(fold(c)))
                    .unwrap_or(Token::Never),
                '[' => {
                    let mut ranges = Vec::new();
                    while let Some(c) = chars.next() {
                        if c == ']' {
                            break;
                        }
                        let start = fold(c);
                        let end = if chars.peek() == Some(&'-') {
                            chars.next();
                            fold(chars.next().unwrap_or('\0'))
                        } else {
                            start
                        };
                        ranges.push((start.min(end), start.max(end)));
                    }
                    Token::Class(ranges)
                }
                _ => Token::Literal(fold(c)),
            });
        }
        Self {
            tokens,
            case_sensitive,
        }
    }

    /// Iterative star retry avoids recursion and exponential branching. Matching
    /// allocates no memory and operates on characters rather than UTF-8 bytes.
    pub fn matches(&self, value: &str) -> bool {
        let mut input = value.chars();
        let mut position = 0;
        let mut retry = None;
        loop {
            if matches!(self.tokens.get(position), Some(Token::Star)) {
                position += 1;
                retry = Some((position, input.clone()));
                continue;
            }
            let c = input.clone().next();
            if position == self.tokens.len() && c.is_none() {
                return true;
            }
            let matched = c.is_some_and(|c| {
                let c = fold(c, self.case_sensitive);
                match self.tokens.get(position) {
                    Some(Token::Any) => true,
                    Some(Token::Literal(expected)) => c == *expected,
                    Some(Token::Class(ranges)) => {
                        ranges.iter().any(|(start, end)| *start <= c && c <= *end)
                    }
                    _ => false,
                }
            });
            if matched {
                input.next();
                position += 1;
            } else if let Some((restart, remaining)) = &mut retry {
                if remaining.next().is_none() {
                    return false;
                }
                input = remaining.clone();
                position = *restart;
            } else {
                return false;
            }
        }
    }
}
fn fold(c: char, sensitive: bool) -> char {
    if sensitive {
        c
    } else {
        c.to_lowercase().next().unwrap()
    }
}
