//! Terminal presentation shared by real fetch/build/install phases.
//! Output stays on stderr; no terminal escapes are emitted into redirected logs.
use crate::theme::ACCENT_RGB;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::{io::IsTerminal, time::Duration};

#[derive(Clone)]
pub struct Progress {
    bars: MultiProgress,
    quiet: bool,
    color: bool,
    terminal: bool,
}
impl Progress {
    pub fn new(quiet: bool) -> Self {
        let terminal = std::io::stderr().is_terminal()
            && std::env::var("TERM").is_ok_and(|term| term != "dumb");
        Self {
            bars: MultiProgress::with_draw_target(if quiet || !terminal {
                ProgressDrawTarget::hidden()
            } else {
                ProgressDrawTarget::stderr_with_hz(12)
            }),
            quiet,
            terminal,
            color: terminal && std::env::var_os("NO_COLOR").is_none(),
        }
    }
    pub fn phase(&self, message: &str) {
        if !self.quiet {
            let arrow = console::style("--->")
                .true_color(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2)
                .bold()
                .force_styling(self.color);
            let message = format!("{arrow}  {}", sanitize(message));
            if self.terminal {
                let _ = self.bars.println(message);
            } else {
                eprintln!("{message}");
            }
        }
    }
    pub fn file(&self, name: &str) -> TransferProgress {
        let bar = self.bars.add(ProgressBar::new_spinner());
        bar.set_prefix(sanitize(name));
        bar.set_style(style(self.color, false));
        bar.set_message("connecting");
        bar.enable_steady_tick(Duration::from_millis(100));
        TransferProgress {
            bar,
            color: self.color,
            manager: self.bars.clone(),
        }
    }
}
pub struct TransferProgress {
    bar: ProgressBar,
    color: bool,
    manager: MultiProgress,
}
impl TransferProgress {
    pub fn start(&self, size: Option<u64>) {
        self.bar.reset();
        self.bar.set_style(style(self.color, size.is_some()));
        if let Some(size) = size {
            self.bar.set_length(size);
        }
        self.bar.set_message("downloading");
    }
    pub fn advance(&self, bytes: u64) {
        self.bar.inc(bytes);
    }
    pub fn message(&self, message: &str) {
        self.bar.set_message(sanitize(message));
    }
    pub fn verified(&self) {
        self.bar.finish_and_clear();
    }
}
impl Drop for TransferProgress {
    fn drop(&mut self) {
        self.bar.finish_and_clear();
        self.manager.remove(&self.bar);
    }
}
fn style(color: bool, known: bool) -> ProgressStyle {
    let template = match (color, known) {
        (true, true) => {
            "  {purple_spinner} {prefix:22!} {purple_bar} {bytes:>10}/{total_bytes:10} {bytes_per_sec:>12} ETA {eta:4} {msg:.dim}"
        }
        (false, true) => {
            "  {spinner} {prefix:22!} {bar:24} {bytes:>10}/{total_bytes:10} {bytes_per_sec:>12} ETA {eta:4} {msg}"
        }
        (true, false) => {
            "  {purple_spinner} {prefix:22!} {bytes:>10} {bytes_per_sec:>12} {msg:.dim}"
        }
        (false, false) => "  {spinner} {prefix:22!} {bytes:>10} {bytes_per_sec:>12} {msg}",
    };
    ProgressStyle::with_template(template)
        .expect("static progress template")
        .with_key(
            "purple_spinner",
            |state: &indicatif::ProgressState, writer: &mut dyn std::fmt::Write| {
                let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
                let frame = frames[(state.elapsed().as_millis() / 100) as usize % frames.len()];
                let _ = write!(
                    writer,
                    "{}",
                    console::style(frame)
                        .true_color(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2)
                        .force_styling(true)
                );
            },
        )
        .with_key(
            "purple_bar",
            |state: &indicatif::ProgressState, writer: &mut dyn std::fmt::Write| {
                let filled = (state.fraction() * 24.0).floor() as usize;
                let _ = write!(
                    writer,
                    "{}{}",
                    console::style("━".repeat(filled))
                        .true_color(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2)
                        .force_styling(true),
                    console::style("─".repeat(24 - filled))
                        .dim()
                        .force_styling(true)
                );
            },
        )
        .progress_chars("━╸─")
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
}
fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn templates_are_valid() {
        for color in [false, true] {
            for known in [false, true] {
                style(color, known);
            }
        }
    }
    #[derive(Clone, Debug, Default)]
    struct Capture(std::sync::Arc<std::sync::Mutex<String>>);
    impl indicatif::TermLike for Capture {
        fn width(&self) -> u16 {
            160
        }
        fn move_cursor_up(&self, _: usize) -> std::io::Result<()> {
            Ok(())
        }
        fn move_cursor_down(&self, _: usize) -> std::io::Result<()> {
            Ok(())
        }
        fn move_cursor_left(&self, _: usize) -> std::io::Result<()> {
            Ok(())
        }
        fn move_cursor_right(&self, _: usize) -> std::io::Result<()> {
            Ok(())
        }
        fn clear_line(&self) -> std::io::Result<()> {
            Ok(())
        }
        fn flush(&self) -> std::io::Result<()> {
            Ok(())
        }
        fn write_str(&self, text: &str) -> std::io::Result<()> {
            self.0.lock().unwrap().push_str(text);
            Ok(())
        }
        fn write_line(&self, text: &str) -> std::io::Result<()> {
            self.write_str(text)
        }
    }
    #[test]
    fn renders_exact_purple_and_plain_mode_has_no_ansi() {
        for color in [false, true] {
            let capture = Capture::default();
            let bar = ProgressBar::with_draw_target(
                Some(100),
                ProgressDrawTarget::term_like(Box::new(capture.clone())),
            );
            bar.set_style(style(color, true));
            bar.set_prefix("curl.tar.xz");
            bar.set_position(50);
            bar.finish();
            let output = capture.0.lock().unwrap();
            assert!(output.contains("curl.tar.xz"));
            assert!(output.contains("ETA"));
            assert_eq!(output.contains("\x1b[38;2;117;23;252m"), color);
            if !color {
                assert!(!output.contains('\x1b'));
            }
        }
    }
    #[test]
    fn untrusted_names_cannot_inject_terminal_controls() {
        assert_eq!(sanitize("hi\x1b[31m\n"), "hi [31m ");
    }
}
