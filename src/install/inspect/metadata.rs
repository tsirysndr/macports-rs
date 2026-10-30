use anyhow::{Context, Result, ensure};
use std::collections::BTreeMap;

/// POSIX time normalized to whole seconds plus nonnegative nanoseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timestamp {
    pub seconds: i64,
    pub nanoseconds: u32,
}

impl Timestamp {
    fn parse(value: &str) -> Result<Self> {
        let (negative, value) = match value.strip_prefix('-') {
            Some(value) => (true, value),
            None => (false, value),
        };
        let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
        ensure!(
            !whole.is_empty()
                && whole.bytes().all(|b| b.is_ascii_digit())
                && fraction.bytes().all(|b| b.is_ascii_digit())
                && (!value.contains('.') || !fraction.is_empty()),
            "invalid archive timestamp"
        );
        let whole: i128 = whole.parse().context("archive timestamp out of range")?;
        // Discard sub-nanosecond precision without floating-point rounding.
        let mut nanos = 0i128;
        for digit in fraction.bytes().take(9) {
            nanos = nanos * 10 + i128::from(digit - b'0');
        }
        nanos *= 10i128.pow(9 - fraction.len().min(9) as u32);
        let seconds = if negative {
            -whole - i128::from(nanos != 0)
        } else {
            whole
        };
        Ok(Self {
            seconds: seconds
                .try_into()
                .context("archive timestamp out of range")?,
            nanoseconds: if negative && nanos != 0 {
                (1_000_000_000 - nanos) as u32
            } else {
                nanos as u32
            },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub uid: u64,
    pub gid: u64,
    pub mtime: Timestamp,
    pub atime: Option<Timestamp>,
}

impl Metadata {
    pub(super) fn read(
        header: &tar::Header,
        attributes: &BTreeMap<String, String>,
    ) -> Result<Self> {
        let id = |key: &str, fallback: fn(&tar::Header) -> std::io::Result<u64>| {
            if let Some(value) = attributes.get(key) {
                ensure!(
                    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
                    "invalid archive {key}"
                );
                value
                    .parse::<u64>()
                    .with_context(|| format!("archive {key} out of range"))
            } else {
                fallback(header).with_context(|| format!("invalid tar {key}"))
            }
        };
        let time = |key: &str| {
            attributes
                .get(key)
                .map(|value| Timestamp::parse(value).with_context(|| format!("invalid PAX {key}")))
                .transpose()
        };
        // ctime cannot be restored by filesystem APIs, but must still be valid.
        time("ctime")?;
        Ok(Self {
            uid: id("uid", tar::Header::uid)?,
            gid: id("gid", tar::Header::gid)?,
            mtime: match time("mtime")? {
                Some(time) => time,
                None => Timestamp {
                    seconds: header
                        .mtime()?
                        .try_into()
                        .context("tar mtime out of range")?,
                    nanoseconds: 0,
                },
            },
            atime: time("atime")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_preserve_fractional_precision_and_normalize_negative_times() {
        for (text, seconds, nanoseconds) in [
            ("0", 0, 0),
            ("123.000000001", 123, 1),
            ("-0.25", -1, 750_000_000),
            ("-12", -12, 0),
            ("-12.9999999999", -13, 1),
            ("9223372036854775807.9", i64::MAX, 900_000_000),
            ("-9223372036854775808", i64::MIN, 0),
        ] {
            assert_eq!(
                Timestamp::parse(text).unwrap(),
                Timestamp {
                    seconds,
                    nanoseconds
                }
            );
        }
        for text in [
            "",
            ".1",
            "1.",
            "+1",
            " 1",
            "1e3",
            "NaN",
            "1.2.3",
            "--1",
            "9223372036854775808",
            "-9223372036854775808.1",
        ] {
            assert!(Timestamp::parse(text).is_err(), "{text}");
        }
    }
}
