//! Human-readable file sizes, in both directions.
//!
//! Form requirements are written the way a person writes them ("between 20 and
//! 50 KB", "under 100kb", "50-100 KB"), so the parser accepts that shape directly
//! and the CLI and UI share one implementation.

use crate::error::{Error, Result};
use crate::target::SizeTarget;

/// Parse a single size such as `100kb`, `1.5 MB`, `204800`, `2 mib`.
///
/// Bare `KB`/`MB` mean 1000-based units, matching how form instructions and
/// file managers state them. `KiB`/`MiB` are 1024-based.
pub fn parse_size(input: &str) -> Result<u64> {
    let s = input.trim().to_ascii_lowercase().replace(' ', "");
    if s.is_empty() {
        return Err(Error::invalid("empty size"));
    }

    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);

    let value: f64 = num.parse().map_err(|_| Error::invalid(format!("'{input}' is not a size")))?;
    if value < 0.0 || !value.is_finite() {
        return Err(Error::invalid(format!("'{input}' is not a size")));
    }

    let multiplier: f64 = match unit {
        "" | "b" => 1.0,
        "k" | "kb" => 1_000.0,
        "m" | "mb" => 1_000_000.0,
        "g" | "gb" => 1_000_000_000.0,
        "kib" => 1_024.0,
        "mib" => 1_048_576.0,
        "gib" => 1_073_741_824.0,
        other => return Err(Error::invalid(format!("unknown unit '{other}'"))),
    };

    Ok((value * multiplier).round() as u64)
}

/// Parse a whole target: `100kb`, `under 100kb`, `50-100kb`, `between 20kb and 50kb`,
/// `exactly 1mb`, `<=500k`.
///
/// A range written with one unit (`50-100kb`) applies that unit to both ends.
pub fn parse_target(input: &str) -> Result<SizeTarget> {
    let raw = input.trim().to_ascii_lowercase();
    let cleaned = raw
        .replace("between", " ")
        .replace("under", " ")
        .replace("below", " ")
        .replace("max", " ")
        .replace("at most", " ")
        .replace("<=", " ")
        .replace('<', " ")
        .replace(" and ", "-")
        .replace(" to ", "-")
        .replace(['–', '—'], "-");
    let exact = raw.contains("exactly") || raw.contains("about") || raw.contains("~");
    let cleaned = cleaned.replace("exactly", " ").replace("about", " ").replace('~', " ");
    let cleaned = cleaned.trim().to_string();

    if let Some((lo, hi)) = split_range(&cleaned) {
        let hi_bytes = parse_size(&hi)?;
        // "50-100kb": the low end inherits the high end's unit when it has none.
        let lo_trim = lo.trim();
        let lo_bytes = if lo_trim.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ' ') {
            let unit: String = hi
                .trim()
                .chars()
                .skip_while(|c| c.is_ascii_digit() || *c == '.' || *c == ' ')
                .collect();
            parse_size(&format!("{lo_trim}{unit}"))?
        } else {
            parse_size(lo_trim)?
        };
        if lo_bytes > hi_bytes {
            return Err(Error::invalid("range is backwards"));
        }
        return Ok(SizeTarget::Range { min: lo_bytes, max: hi_bytes });
    }

    let bytes = parse_size(&cleaned)?;
    if exact {
        Ok(SizeTarget::Approx { bytes, tol: 0.1 })
    } else {
        Ok(SizeTarget::AtMost(bytes))
    }
}

/// Split on a hyphen that separates two numbers, not one inside a unit.
fn split_range(s: &str) -> Option<(String, String)> {
    for (i, c) in s.char_indices() {
        if c == '-' && i > 0 && i + 1 < s.len() {
            let before = &s[..i];
            let after = &s[i + 1..];
            if before.chars().any(|c| c.is_ascii_digit())
                && after.chars().any(|c| c.is_ascii_digit())
            {
                return Some((before.to_string(), after.to_string()));
            }
        }
    }
    None
}

/// Render a size the way the UI shows it: `87 KB`, `1.5 MB`, `640 B`.
pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1_000.0;
    const MB: f64 = 1_000_000.0;
    const GB: f64 = 1_000_000_000.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{} KB", (b / KB).round() as u64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_sizes() {
        assert_eq!(parse_size("100kb").unwrap(), 100_000);
        assert_eq!(parse_size("1.5 MB").unwrap(), 1_500_000);
        assert_eq!(parse_size("204800").unwrap(), 204_800);
        assert_eq!(parse_size("2 MiB").unwrap(), 2_097_152);
        assert!(parse_size("banana").is_err());
    }

    #[test]
    fn parses_the_shapes_forms_actually_use() {
        assert_eq!(parse_target("under 100kb").unwrap(), SizeTarget::AtMost(100_000));
        assert_eq!(
            parse_target("50-100kb").unwrap(),
            SizeTarget::Range { min: 50_000, max: 100_000 }
        );
        assert_eq!(
            parse_target("between 20 and 50 KB").unwrap(),
            SizeTarget::Range { min: 20_000, max: 50_000 }
        );
        assert_eq!(
            parse_target("20kb to 50kb").unwrap(),
            SizeTarget::Range { min: 20_000, max: 50_000 }
        );
        assert!(matches!(parse_target("exactly 1mb").unwrap(), SizeTarget::Approx { .. }));
        assert!(parse_target("100-50kb").is_err());
    }

    #[test]
    fn formats_for_display() {
        assert_eq!(format_size(87_000), "87 KB");
        assert_eq!(format_size(1_500_000), "1.5 MB");
        assert_eq!(format_size(640), "640 B");
    }
}
