use super::Phase;
use crate::theme::ACCENT_RGB;
use std::io;
use tokio::io::{AsyncWrite, AsyncWriteExt};

pub(super) fn prefix(port: &str, phase: Phase, color: bool) -> String {
    let port: String = port
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    format!(
        "{} ",
        console::style(format!("-> {} {port}:", phase.label()))
            .true_color(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2)
            .force_styling(color)
    )
}
pub(super) struct Log<'a, W> {
    output: &'a mut W,
    prefix: &'a [u8],
    at_start: bool,
    after_cr: bool,
    stream: Option<bool>,
}
impl<'a, W: AsyncWrite + Unpin> Log<'a, W> {
    pub fn new(output: &'a mut W, prefix: &'a str) -> Self {
        Self {
            output,
            prefix: prefix.as_bytes(),
            at_start: true,
            after_cr: false,
            stream: None,
        }
    }
    pub async fn write(&mut self, stderr: bool, bytes: &[u8]) -> io::Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let mut rendered = Vec::with_capacity(bytes.len() + self.prefix.len());
        if self.stream.is_some_and(|previous| previous != stderr) {
            if !self.at_start {
                rendered.push(b'\n');
                self.at_start = true;
            }
            self.after_cr = false;
        }
        self.stream = Some(stderr);
        for &byte in bytes {
            if self.after_cr {
                self.after_cr = false;
                if byte == b'\n' {
                    continue;
                }
            }
            if self.at_start {
                rendered.extend_from_slice(self.prefix);
                self.at_start = false;
            }
            match byte {
                b'\r' => {
                    rendered.push(b'\n');
                    self.at_start = true;
                    self.after_cr = true;
                }
                b'\n' => {
                    rendered.push(byte);
                    self.at_start = true;
                }
                _ => rendered.push(byte),
            }
        }
        self.output.write_all(&rendered).await?;
        self.output.flush().await
    }
    pub async fn finish(&mut self) -> io::Result<()> {
        if !self.at_start {
            self.output.write_all(b"\n").await?;
            self.at_start = true;
        }
        self.output.flush().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn prefixes_lines_without_waiting_for_a_newline() {
        let mut output = Vec::new();
        let mut log = Log::new(&mut output, "-> building git: ");
        log.write(false, b"comp").await.unwrap();
        assert_eq!(log.output.as_slice(), b"-> building git: comp");
        log.write(false, b"iling\nlinking\n").await.unwrap();
        log.finish().await.unwrap();
        assert_eq!(
            output,
            b"-> building git: compiling\n-> building git: linking\n"
        );
    }
    #[tokio::test]
    async fn partial_stdout_and_stderr_get_separate_labeled_lines() {
        let mut output = Vec::new();
        let mut log = Log::new(&mut output, "-> building git: ");
        log.write(false, b"working").await.unwrap();
        log.write(true, b"warning\n").await.unwrap();
        log.write(false, b"done").await.unwrap();
        log.finish().await.unwrap();
        assert_eq!(
            output,
            b"-> building git: working\n-> building git: warning\n-> building git: done\n"
        );
    }
    #[tokio::test]
    async fn handles_crlf_progress_updates_empty_lines_and_non_utf8() {
        let mut output = Vec::new();
        let mut log = Log::new(&mut output, "-> building git: ");
        log.write(false, b"10%\r").await.unwrap();
        log.write(false, b"\n20%\r30%\n\n\xff").await.unwrap();
        log.finish().await.unwrap();
        assert_eq!(output, b"-> building git: 10%\n-> building git: 20%\n-> building git: 30%\n-> building git: \n-> building git: \xff\n");
    }
    #[test]
    fn plain_prefix_has_no_color_or_control_characters_from_port_names() {
        assert_eq!(prefix("git", Phase::Build, false), "-> building git: ");
        assert_eq!(
            prefix("git", Phase::Configure, false),
            "-> configuring git: "
        );
        assert!(!prefix("git\n\x1b", Phase::Build, false).contains(['\n', '\x1b']));
    }
}
