pub(super) fn bytesize(bytes: u64, unit: Option<&str>) -> String {
    let unit = match unit.unwrap_or("") {
        "" if bytes > 1_073_741_824 => "GiB",
        "" if bytes > 1_048_576 => "MiB",
        "" if bytes > 1024 => "KiB",
        "" => "B",
        "K" | "Ki" => "KiB",
        "k" => "kB",
        "Mi" => "MiB",
        "M" => "MB",
        "Gi" => "GiB",
        "G" => "GB",
        unit => unit,
    };
    let divisor = match unit {
        "KiB" => 1024.0,
        "kB" => 1000.0,
        "MiB" => 1_048_576.0,
        "MB" => 1_000_000.0,
        "GiB" => 1_073_741_824.0,
        "GB" => 1_000_000_000.0,
        "B" => return format!("{bytes} B"),
        _ => {
            eprintln!("Warning: Unknown file size unit '{unit}' specified");
            return format!("{bytes} B");
        }
    };
    let value = bytes as f64 / divisor;
    if value.fract() == 0.0 {
        format!("{value:.1} {unit}")
    } else {
        format!("{value:.2} {unit}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn size_units_preserve_upstream_thresholds_and_abbreviations() {
        assert_eq!(bytesize(1024, None), "1024 B");
        assert_eq!(bytesize(1025, None), "1.00 KiB");
        assert_eq!(bytesize(1_048_576, None), "1024.0 KiB");
        assert_eq!(bytesize(1_073_741_824, None), "1024.0 MiB");
        for (unit, expected) in [
            ("K", "1.0 KiB"),
            ("Ki", "1.0 KiB"),
            ("k", "1.02 kB"),
            ("B", "1024 B"),
        ] {
            assert_eq!(bytesize(1024, Some(unit)), expected);
        }
        assert_eq!(bytesize(1_000_000, Some("M")), "1.0 MB");
        assert_eq!(bytesize(0, Some("Gi")), "0.0 GiB");
    }
}
