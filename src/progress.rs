//! Terminal presentation shared by real fetch/build/install phases.
//! Output stays on stderr; no terminal escapes are emitted into redirected logs.
use crate::theme::{PRIMARY_RGB, SECONDARY_RGB};
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::{
    io::IsTerminal,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone)]
pub struct Progress {
    bars: MultiProgress,
    quiet: bool,
    color: bool,
    terminal: bool,
    pauses: Arc<Mutex<usize>>,
}
impl Progress {
    pub fn new(quiet: bool) -> Self {
        let terminal = std::io::stderr().is_terminal()
            && std::env::var_os("TERM").is_none_or(|term| term != "dumb");
        Self {
            bars: MultiProgress::with_draw_target(if quiet || !terminal {
                ProgressDrawTarget::hidden()
            } else {
                ProgressDrawTarget::stderr_with_hz(12)
            }),
            quiet,
            terminal,
            pauses: Arc::new(Mutex::new(0)),
            color: terminal && std::env::var_os("NO_COLOR").is_none(),
        }
    }
    pub(crate) fn color_enabled(&self) -> bool {
        self.color
    }
    /// Hide progress redraws while live compiler output owns the terminal.
    pub(crate) fn pause_bars(&self) -> PausedBars {
        let mut count = self.pauses.lock().expect("progress pause lock");
        if *count == 0 {
            self.bars.set_draw_target(ProgressDrawTarget::hidden());
        }
        *count += 1;
        PausedBars(self.clone())
    }
    pub fn phase(&self, message: &str) {
        if !self.quiet {
            let arrow = console::style("--->")
                .true_color(PRIMARY_RGB.0, PRIMARY_RGB.1, PRIMARY_RGB.2)
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
        if self.terminal && !self.quiet {
            bar.enable_steady_tick(Duration::from_millis(100));
        }
        TransferProgress {
            bar,
            color: self.color,
            manager: self.bars.clone(),
        }
    }
}
pub(crate) struct PausedBars(Progress);
impl Drop for PausedBars {
    fn drop(&mut self) {
        let mut count = self.0.pauses.lock().expect("progress pause lock");
        *count -= 1;
        if *count == 0 && self.0.terminal && !self.0.quiet {
            self.0
                .bars
                .set_draw_target(ProgressDrawTarget::stderr_with_hz(12));
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
            "  {transfer_spinner} {prefix:22!} {transfer_bar} {bytes:>10}/{total_bytes:10} {bytes_per_sec:>12} ETA {eta:4} {msg:.dim}"
        }
        (false, true) => {
            "  {spinner} {prefix:22!} {bar:24} {bytes:>10}/{total_bytes:10} {bytes_per_sec:>12} ETA {eta:4} {msg}"
        }
        (true, false) => {
            "  {transfer_spinner} {prefix:22!} {bytes:>10} {bytes_per_sec:>12} {msg:.dim}"
        }
        (false, false) => "  {spinner} {prefix:22!} {bytes:>10} {bytes_per_sec:>12} {msg}",
    };
    ProgressStyle::with_template(template)
        .expect("static progress template")
        .with_key(
            "transfer_spinner",
            |state: &indicatif::ProgressState, writer: &mut dyn std::fmt::Write| {
                let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
                let frame = frames[(state.elapsed().as_millis() / 100) as usize % frames.len()];
                let _ = write!(
                    writer,
                    "{}",
                    console::style(frame)
                        .true_color(SECONDARY_RGB.0, SECONDARY_RGB.1, SECONDARY_RGB.2)
                        .force_styling(true)
                );
            },
        )
        .with_key(
            "transfer_bar",
            |state: &indicatif::ProgressState, writer: &mut dyn std::fmt::Write| {
                let filled = (state.fraction() * 24.0).floor() as usize;
                let _ = write!(
                    writer,
                    "{}{}",
                    console::style("━".repeat(filled))
                        .true_color(SECONDARY_RGB.0, SECONDARY_RGB.1, SECONDARY_RGB.2)
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
    fn renders_rocksky_cyan_and_plain_mode_has_no_ansi() {
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
            assert_eq!(output.contains("\x1b[38;2;0;198;232m"), color);
            if !color {
                assert!(!output.contains('\x1b'));
            }
        }
    }
    #[test]
    fn untrusted_names_cannot_inject_terminal_controls() {
        assert_eq!(sanitize("hi\x1b[31m\n"), "hi [31m ");
    }

    #[test]
    fn concurrent_transfers_share_terminal_rows_and_preserve_phase_messages() {
        let capture = Capture::default();
        let progress = Progress {
            bars: MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(
                capture.clone(),
            ))),
            quiet: false,
            color: false,
            terminal: true,
            pauses: Arc::new(Mutex::new(0)),
        };
        let first = progress.file("curl.tar.xz");
        let second = progress.clone().file("git.tar.gz");
        first.start(Some(100));
        second.start(None);
        first.advance(50);
        second.advance(20);
        capture.0.lock().unwrap().clear();
        progress.phase("Verifying a completed transfer");
        let output = capture.0.lock().unwrap().clone();
        assert!(output.contains("curl.tar.xz"));
        assert!(output.contains("git.tar.gz"));
        assert!(output.contains("Verifying a completed transfer"));
        assert!(output.contains("ETA"));
        assert!(!output.contains('\x1b'));

        first.verified();
        drop(first);
        capture.0.lock().unwrap().clear();
        progress.phase("One transfer remains");
        let output = capture.0.lock().unwrap().clone();
        assert!(!output.contains("curl.tar.xz"));
        assert!(output.contains("git.tar.gz"));
        second.verified();
    }
}
