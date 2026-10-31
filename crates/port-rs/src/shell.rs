//! The interactive shell's line editor, as upstream's readline setup:
//! history in `~/.macports/history` (libedit's format), actions completed
//! first, port names after, and file names for paths.
use rustyline::{
    Context, Editor, Helper,
    completion::{Completer, FilenameCompleter, Pair},
    highlight::Highlighter,
    hint::Hinter,
    history::DefaultHistory,
    validate::Validator,
};
use std::{
    cell::OnceCell,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const COOKIE: &str = "_HiStOrY_V2_";

pub(crate) struct Shell {
    index: Option<PathBuf>,
    names: OnceCell<Vec<String>>,
    files: FilenameCompleter,
}

impl Shell {
    fn names(&self) -> &[String] {
        self.names
            .get_or_init(|| port_rs::commands::port_names(self.index.as_deref()))
    }
}

/// Where the completed word starts: after whitespace or `;`.
fn word_start(line: &str) -> usize {
    line.rfind(|c: char| c.is_whitespace() || c == ';')
        .map_or(0, |index| index + 1)
}

impl Completer for Shell {
    type Candidate = Pair;
    fn complete(
        &self,
        line: &str,
        pos: usize,
        context: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let start = word_start(&line[..pos]);
        let word = &line[start..pos];
        if word.starts_with('~') || word.contains(['/', '.']) {
            return self.files.complete(line, pos, context);
        }
        let before = line[..start].trim_end();
        let lower = word.to_lowercase();
        let mut candidates: Vec<String> = if before.is_empty() || before.ends_with(';') {
            port_rs::commands::ACTIONS
                .iter()
                .filter(|action| action.starts_with(&lower))
                .map(|action| action.to_string())
                .collect()
        } else {
            self.names()
                .iter()
                .filter(|name| name.to_lowercase().starts_with(&lower))
                .cloned()
                .collect()
        };
        candidates.sort();
        Ok((
            start,
            candidates
                .into_iter()
                .map(|candidate| Pair {
                    display: candidate.clone(),
                    replacement: candidate,
                })
                .collect(),
        ))
    }
}
impl Hinter for Shell {
    type Hint = String;
}
impl Highlighter for Shell {}
impl Validator for Shell {}
impl Helper for Shell {}

fn history_path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join(".macports/history"))
}

/// libedit stores lines with `strvis`-style octal escapes.
fn escape(line: &str) -> String {
    let mut escaped = String::new();
    for character in line.chars() {
        match character {
            ' ' | '\t' | '\n' | '\\' => escaped.push_str(&format!("\\{:03o}", character as u32)),
            other => escaped.push(other),
        }
    }
    escaped
}

fn unescape(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut text = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\'
            && let Some(digits) = line.get(index + 1..index + 4)
            && let Ok(value) = u8::from_str_radix(digits, 8)
        {
            text.push(value);
            index += 4;
        } else {
            text.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&text).into_owned()
}

fn load(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|line| *line != COOKIE && !line.is_empty())
        .map(unescape)
        .collect()
}

fn append(path: &Path, line: &str) {
    let fresh = !path.exists();
    if let Some(directory) = path.parent() {
        let _ = fs::create_dir_all(directory);
    }
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        if fresh {
            let _ = writeln!(file, "{COOKIE}");
        }
        let _ = writeln!(file, "{}", escape(line));
    }
}

pub(crate) struct LineEditor {
    editor: Editor<Shell, DefaultHistory>,
    history: Option<PathBuf>,
}

impl LineEditor {
    pub(crate) fn new(index: Option<PathBuf>) -> Option<Self> {
        let mut editor = Editor::<Shell, DefaultHistory>::new().ok()?;
        editor.set_helper(Some(Shell {
            index,
            names: OnceCell::new(),
            files: FilenameCompleter::new(),
        }));
        let history = history_path();
        if let Some(path) = &history {
            for line in load(path) {
                let _ = editor.add_history_entry(line);
            }
        }
        Some(Self { editor, history })
    }

    /// The next non-empty line, or `None` at end of input.
    pub(crate) fn read(&mut self, prompt: &str) -> Option<String> {
        loop {
            match self.editor.readline(prompt) {
                Ok(line) => {
                    let line = line.trim().to_string();
                    if line.is_empty() {
                        continue;
                    }
                    let _ = self.editor.add_history_entry(line.as_str());
                    if let Some(path) = &self.history {
                        append(path, &line);
                    }
                    return Some(line);
                }
                Err(rustyline::error::ReadlineError::Interrupted) => continue,
                Err(_) => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_round_trips_libedit_escapes() {
        let line = "install zlib +universal\tx\\y";
        assert_eq!(escape(line), "install\\040zlib\\040+universal\\011x\\134y");
        assert_eq!(unescape(&escape(line)), line);
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".macports/history");
        append(&path, "info zlib");
        append(&path, "deps curl");
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .starts_with("_HiStOrY_V2_\n")
        );
        assert_eq!(load(&path), ["info zlib", "deps curl"]);
    }

    #[test]
    fn completion_offers_actions_first_then_port_names() {
        let temp = tempfile::tempdir().unwrap();
        let index = temp.path().join("PortIndex");
        let mut text = String::new();
        for name in ["zlib", "zstd", "curl"] {
            let body = format!("name {name} version 1");
            text.push_str(&format!("{name} {}\n{body}\n", body.len() + 1));
        }
        fs::write(&index, text).unwrap();
        let shell = Shell {
            index: Some(index),
            names: OnceCell::new(),
            files: FilenameCompleter::new(),
        };
        let history = DefaultHistory::new();
        let context = Context::new(&history);
        let complete = |line: &str| {
            let (start, pairs) = shell.complete(line, line.len(), &context).unwrap();
            (
                start,
                pairs
                    .into_iter()
                    .map(|pair| pair.replacement)
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            complete("inst"),
            (0, vec!["install".into(), "installed".into()])
        );
        assert_eq!(complete("info z"), (5, vec!["zlib".into(), "zstd".into()]));
        assert_eq!(complete("info zlib; dep").1, ["dependents", "deps"]);
    }
}
