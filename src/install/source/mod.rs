//! Source distribution requests, transport, and checksum verification.
use std::path::PathBuf;
pub mod checksum;
mod download;
pub use download::run as download;
/// Per-port `fetch.*` transport options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    pub credentials: Option<(String, String)>,
    pub ignore_sslcert: bool,
    pub remote_time: bool,
    pub user_agent: Option<String>,
    /// Prefix for "Attempting to fetch" notices; empty disables them.
    pub prefix: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub filename: String,
    pub destination: PathBuf,
    pub urls: Vec<String>,
    /// The site each URL was assembled from, for progress notices.
    pub sites: Vec<String>,
}
/// Match MacPorts' percent_encode character-code formatting, including its
/// historical non-ASCII behavior; do not normalize the supplied mirror URL.
pub fn percent_encode(text: &str) -> String {
    let mut result = String::new();
    for character in text.chars() {
        if character.is_ascii_alphanumeric() || "-._~".contains(character) {
            result.push(character);
        } else {
            let hex = format!("{:02X}", character as u32);
            for pair in hex.as_bytes().chunks(2) {
                result.push('%');
                for byte in pair {
                    result.push(*byte as char);
                }
            }
        }
    }
    result
}
pub fn assemble_url(site: &str, filename: &str) -> String {
    format!(
        "{site}{}{}",
        if site.ends_with('/') { "" } else { "/" },
        percent_encode(filename)
    )
}
