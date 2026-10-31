//! Shared Rocksky CLI palette for help output and live progress.
use clap::builder::styling::{RgbColor, Styles};

// Matches ../rocksky/apps/cli/src/theme.ts.
pub const PRIMARY_RGB: (u8, u8, u8) = (0x00, 0xe8, 0xc6);
pub const SECONDARY_RGB: (u8, u8, u8) = (0x00, 0xc6, 0xe8);
pub const ACCENT_RGB: (u8, u8, u8) = (0x82, 0x64, 0xff);
pub const HIGHLIGHT_RGB: (u8, u8, u8) = (0x64, 0xe8, 0x82);
pub const ERROR_RGB: (u8, u8, u8) = (0xff, 0x64, 0x64);

pub fn help_styles() -> Styles {
    let primary = RgbColor(PRIMARY_RGB.0, PRIMARY_RGB.1, PRIMARY_RGB.2).on_default();
    let secondary = RgbColor(SECONDARY_RGB.0, SECONDARY_RGB.1, SECONDARY_RGB.2).on_default();
    let accent = RgbColor(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).on_default();
    let highlight = RgbColor(HIGHLIGHT_RGB.0, HIGHLIGHT_RGB.1, HIGHLIGHT_RGB.2).on_default();
    let error = RgbColor(ERROR_RGB.0, ERROR_RGB.1, ERROR_RGB.2).on_default();
    Styles::styled()
        .header(primary.bold())
        .usage(primary.bold())
        .literal(secondary)
        .placeholder(accent)
        .valid(highlight)
        .invalid(error)
        .error(error.bold())
}
