//! Minimal RFC 3339 UTC formatting from Unix epoch milliseconds.
//!
//! Hand-rolled (civil-from-days algorithm, Howard Hinnant's `civil_from_days`)
//! to keep the dependency tree small; validated against known epochs below.

/// Format Unix epoch milliseconds as `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC).
#[must_use]
pub fn format_utc_from_unix_ms(unix_ms: u64) -> String {
    let secs = unix_ms / 1000;
    let millis = unix_ms % 1000;
    let days = secs / 86_400;
    let second_of_day = secs % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    let (hour, minute, second) = (
        second_of_day / 3_600,
        (second_of_day % 3_600) / 60,
        second_of_day % 60,
    );
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Convert days since 1970-01-01 to a proleptic Gregorian (year, month, day).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_1970_start() {
        assert_eq!(format_utc_from_unix_ms(0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn known_timestamps() {
        // 2024-02-29T00:00:00Z (leap day) = 1_709_164_800 s.
        assert_eq!(
            format_utc_from_unix_ms(1_709_164_800_000),
            "2024-02-29T00:00:00.000Z"
        );
        // 2024-03-01T00:00:00Z.
        assert_eq!(
            format_utc_from_unix_ms(1_709_251_200_000),
            "2024-03-01T00:00:00.000Z"
        );
        // 2026-10-22T12:34:56.789Z (documentation revision 0.2 date).
        assert_eq!(
            format_utc_from_unix_ms(1_792_672_496_789),
            "2026-10-22T12:34:56.789Z"
        );
        // Year boundary: 2025-01-01T00:00:00Z = 1_735_689_600 s.
        assert_eq!(
            format_utc_from_unix_ms(1_735_689_600_000),
            "2025-01-01T00:00:00.000Z"
        );
        // End of day: 2000-02-29T23:59:59.999Z (leap year 2000).
        assert_eq!(
            format_utc_from_unix_ms(951_868_799_999),
            "2000-02-29T23:59:59.999Z"
        );
    }

    #[test]
    fn output_shape_is_stable() {
        let s = format_utc_from_unix_ms(1_792_672_496_789);
        assert_eq!(s.len(), 24);
        assert!(s.starts_with("20"));
        assert!(s.ends_with('Z'));
        assert_eq!(&s[10..11], "T");
        assert_eq!(&s[19..20], ".");
    }
}
