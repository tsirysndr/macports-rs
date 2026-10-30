use std::cmp::Ordering;

// MacPorts compares alternating ASCII numeric/alphabetic segments, ignoring
// separators. Numbers sort after letters; leading numeric zeroes are ignored.
pub(super) fn compare(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    while !a.is_empty() && !b.is_empty() {
        a = &a[a.iter().take_while(|c| !c.is_ascii_alphanumeric()).count()..];
        b = &b[b.iter().take_while(|c| !c.is_ascii_alphanumeric()).count()..];
        let numeric = a.first().is_some_and(u8::is_ascii_digit);
        if numeric && !b.first().is_some_and(u8::is_ascii_digit) {
            return Ordering::Greater;
        }
        if a.first().is_some_and(u8::is_ascii_alphabetic)
            && b.first().is_some_and(u8::is_ascii_digit)
        {
            return Ordering::Less;
        }
        let segment = |bytes: &[u8]| {
            bytes
                .iter()
                .take_while(|c| {
                    if numeric {
                        c.is_ascii_digit()
                    } else {
                        c.is_ascii_alphabetic()
                    }
                })
                .count()
        };
        let (alen, blen) = (segment(a), segment(b));
        let (mut x, mut y) = (&a[..alen], &b[..blen]);
        if numeric {
            x = &x[x.iter().take_while(|c| **c == b'0').count()..];
            y = &y[y.iter().take_while(|c| **c == b'0').count()..];
            let length = x.len().cmp(&y.len());
            if length != Ordering::Equal {
                return length;
            }
        }
        // Upstream ignores an unmatched alphabetic suffix within a segment.
        for (x, y) in x.iter().zip(y) {
            if x != y {
                return x.cmp(y);
            }
        }
        a = &a[alen..];
        b = &b[blen..];
    }
    (!a.is_empty()).cmp(&!b.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn macports_version_segments_are_not_semver_or_dictionary_order() {
        for (a, b, expected) in [
            ("8.9", "8.10", Ordering::Less),
            ("1.01", "1.1", Ordering::Equal),
            ("1-2", "1.2", Ordering::Equal),
            ("1a", "1.1", Ordering::Less),
            ("1.0", "1.0a", Ordering::Less),
            ("1a", "1abc", Ordering::Equal),
            ("1.0", "1.0.", Ordering::Less),
            ("1.0.", "1.0_", Ordering::Equal),
            (
                "999999999999999999999999",
                "1000000000000000000000000",
                Ordering::Less,
            ),
            ("1A", "1a", Ordering::Less),
        ] {
            assert_eq!(compare(a, b), expected, "{a} vs {b}");
            assert_eq!(compare(b, a), expected.reverse(), "{b} vs {a}");
        }
    }
}
