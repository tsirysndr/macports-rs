//! Tcl string matching, independent of filesystem glob semantics.
#[derive(Debug)]
pub struct Glob {
    pattern: Vec<char>,
    case_sensitive: bool,
}
impl Glob {
    pub fn new(pattern: &str, case_sensitive: bool) -> Self {
        Self {
            pattern: pattern.chars().collect(),
            case_sensitive,
        }
    }
    fn fold(&self, value: char) -> char {
        if self.case_sensitive {
            value
        } else {
            value.to_lowercase().next().unwrap()
        }
    }
    /// Iterative star retry avoids recursive/exponential branching. Matching
    /// allocates no memory and consumes Unicode characters rather than bytes.
    pub fn matches(&self, value: &str) -> bool {
        let mut input = value.chars();
        let mut position = 0;
        let mut retry = None;
        loop {
            if self.pattern.get(position) == Some(&'*') {
                while self.pattern.get(position) == Some(&'*') {
                    position += 1;
                }
                retry = Some((position, input.clone()));
                continue;
            }
            let next = input.clone().next();
            if position == self.pattern.len() && next.is_none() {
                return true;
            }
            if let Some(end) = next.and_then(|value| self.character(position, self.fold(value))) {
                input.next();
                position = end;
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
    fn character(&self, position: usize, value: char) -> Option<usize> {
        match *self.pattern.get(position)? {
            '?' => Some(position + 1),
            '\\' => (self.fold(*self.pattern.get(position + 1)?) == value).then_some(position + 2),
            '[' => {
                let mut offset = position + 1;
                loop {
                    let first = *self.pattern.get(offset)?;
                    if first == ']' {
                        return None;
                    }
                    offset += 1;
                    let start = self.fold(first);
                    let end = if self.pattern.get(offset) == Some(&'-') {
                        offset += 1;
                        let end = self.fold(*self.pattern.get(offset)?);
                        offset += 1;
                        end
                    } else {
                        start
                    };
                    if start.min(end) <= value && value <= start.max(end) {
                        // Tcl stops interpreting the class as soon as an entry
                        // matches, then skips to the next closing bracket (or
                        // pattern end). This matters for malformed classes.
                        while let Some(character) = self.pattern.get(offset) {
                            offset += 1;
                            if *character == ']' {
                                break;
                            }
                        }
                        return Some(offset);
                    }
                }
            }
            expected => (self.fold(expected) == value).then_some(position + 1),
        }
    }
}
