//! Shared accent for help output and live progress.
use clap::builder::styling::{RgbColor, Style, Styles};

pub const ACCENT_RGB: (u8, u8, u8) = (0x75, 0x17, 0xfc);

pub fn help_styles() -> Styles {
    let accent = RgbColor(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).on_default();
    Styles::styled()
        .header(accent.bold())
        .usage(accent.bold())
        .literal(accent.bold())
        .placeholder(Style::new().dimmed())
}
