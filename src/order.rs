//! Dictionary ordering for catalog results: numeric runs, then case/zero tie-breaks.
use std::cmp::Ordering;

pub(crate) fn dictionary(mut left: &str, mut right: &str) -> Ordering {
    let mut secondary = Ordering::Equal;
    while !left.is_empty() && !right.is_empty() {
        if left.as_bytes()[0].is_ascii_digit() && right.as_bytes()[0].is_ascii_digit() {
            let l = left.bytes().take_while(u8::is_ascii_digit).count();
            let r = right.bytes().take_while(u8::is_ascii_digit).count();
            let a = left[..l].trim_start_matches('0');
            let b = right[..r].trim_start_matches('0');
            let primary = a.len().cmp(&b.len()).then_with(|| a.cmp(b));
            if primary != Ordering::Equal {
                return primary;
            }
            secondary = secondary.then_with(|| l.cmp(&r));
            left = &left[l..];
            right = &right[r..];
        } else {
            let a = left.chars().next().unwrap();
            let b = right.chars().next().unwrap();
            let primary = a.to_lowercase().next().cmp(&b.to_lowercase().next());
            if primary != Ordering::Equal {
                return primary;
            }
            if secondary == Ordering::Equal {
                if a.is_uppercase() && b.is_lowercase() {
                    secondary = Ordering::Less;
                } else if a.is_lowercase() && b.is_uppercase() {
                    secondary = Ordering::Greater;
                }
            }
            left = &left[a.len_utf8()..];
            right = &right[b.len_utf8()..];
        }
    }
    left.len().cmp(&right.len()).then(secondary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_compared_without_integer_overflow() {
        for (a, b) in [
            ("python9", "python10"),
            (
                "x99999999999999999999999999",
                "x100000000000000000000000000",
            ),
            ("x0", "x00"),
            ("x1", "x01"),
            ("x0009", "x10"),
            ("x2a", "x2b"),
        ] {
            assert_eq!(dictionary(a, b), Ordering::Less, "{a} vs {b}");
            assert_eq!(dictionary(b, a), Ordering::Greater);
        }
    }

    #[test]
    fn case_and_zeros_are_secondary_to_the_whole_name() {
        for (a, b) in [
            ("a", "B"),
            ("A", "a"),
            ("a1", "A2"),
            ("x01a", "x1b"),
            ("x01a", "x1B"),
            ("A01", "a1"),
            ("_x", "Ax"),
            ("x", "x0"),
            ("É2", "é10"),
            ("é", "Éa"),
        ] {
            // Numeric/case ties are considered only after primary characters match.
            assert_eq!(dictionary(a, b), Ordering::Less, "{a} vs {b}");
        }
    }

    #[test]
    fn comparator_is_consistent_on_mixed_names() {
        let names = [
            "", "x", "X", "x0", "x00", "x01", "x1", "X01", "x1a", "x01B", "x2", "x10", "é", "É",
        ];
        for a in names {
            assert_eq!(dictionary(a, a), Ordering::Equal);
            for b in names {
                assert_eq!(dictionary(a, b), dictionary(b, a).reverse());
                for c in names {
                    if dictionary(a, b).is_le() && dictionary(b, c).is_le() {
                        assert!(dictionary(a, c).is_le(), "{a} <= {b} <= {c}");
                    }
                }
            }
        }
    }
}
