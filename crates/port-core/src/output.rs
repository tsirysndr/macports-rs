//! Shared terminal text layout helpers.
/// The width to wrap output to: the terminal's, or unlimited when stdout is
/// not a terminal, as upstream's `term_get_size`.
pub fn terminal_width() -> usize {
    let term = console::Term::stdout();
    if term.is_term() {
        usize::from(term.size().1).max(40)
    } else {
        usize::MAX
    }
}

pub fn wrap(text: &str, prefix: &str, indent: usize, width: usize) -> String {
    // Off a terminal upstream prints the text as is, without wrapping.
    if width == usize::MAX {
        return format!("{prefix}{text}");
    }
    let continuation = " ".repeat(indent);
    let mut lines = Vec::new();
    let mut current = prefix.to_owned();
    let mut has_word = false;
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            lines.push(current);
            current = continuation.clone();
            has_word = false;
        }
        for word in line.split_whitespace() {
            if has_word && current.chars().count() + 1 + word.chars().count() > width {
                lines.push(current);
                current = continuation.clone();
                has_word = false;
            }
            if has_word {
                current.push(' ');
            }
            current.push_str(word);
            has_word = true;
        }
    }
    lines.push(current);
    lines.join("\n")
}
