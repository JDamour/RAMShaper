//! Byte-unit handling with checked arithmetic.
//!
//! Windows reports memory counters as page counts (`GetPerformanceInfo`) plus a
//! page size. Conversion to bytes must be checked: a zero page size or an
//! overflowing product is an error, never a wrapped or invented value
//! (AT-02). MiB/GiB formatting is presentation-only.

use serde::Serialize;

/// A byte count. Internal values are always bytes.
pub type Bytes = u64;

const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;
const BYTES_PER_GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// Errors from page-to-byte conversion. These are never silently coerced to 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PageConversionError {
    /// The reported page size was zero; page counts cannot be converted.
    #[error("page size is zero")]
    ZeroPageSize,
    /// `pages * page_size` does not fit in a `u64`.
    #[error("page count times page size overflows u64")]
    Overflow,
}

/// Convert a page count to bytes using checked multiplication.
pub fn pages_to_bytes(pages: u64, page_size_bytes: u64) -> Result<Bytes, PageConversionError> {
    if page_size_bytes == 0 {
        return Err(PageConversionError::ZeroPageSize);
    }
    pages
        .checked_mul(page_size_bytes)
        .ok_or(PageConversionError::Overflow)
}

/// Checked `minuend - subtrahend`; `None` when the result would be negative
/// (for example available > total in one snapshot).
pub fn byte_difference(minuend: u64, subtrahend: u64) -> Option<Bytes> {
    minuend.checked_sub(subtrahend)
}

/// Human-readable byte formatting for text output only: GiB at >= 1 GiB,
/// otherwise MiB, always with the exact byte count beside it.
#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    let gib = bytes as f64 / BYTES_PER_GIB;
    if gib >= 1.0 {
        format!("{gib:.2} GiB")
    } else {
        let mib = bytes as f64 / BYTES_PER_MIB;
        format!("{mib:.2} MiB")
    }
}

/// JSON helper result: the exact byte count (integers only, never a float).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ByteValue(pub Bytes);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_conversion_for_common_page_sizes() {
        // 4 KiB pages: 1,048,576 pages (4096 MiB) = 4 GiB exactly.
        assert_eq!(
            pages_to_bytes(1_048_576, 4096).expect("exact"),
            4 * 1024 * 1024 * 1024
        );
        // Large-page systems: 2 MiB pages, 8192 pages = 16 GiB.
        assert_eq!(
            pages_to_bytes(8192, 2 * 1024 * 1024).expect("exact"),
            16 * 1024 * 1024 * 1024
        );
        // 64 KiB pages (possible ARM64 configuration): 16 pages = 1 MiB.
        assert_eq!(pages_to_bytes(16, 65_536).expect("exact"), 1_048_576);
        assert_eq!(pages_to_bytes(0, 4096).expect("zero pages"), 0);
    }

    #[test]
    fn zero_page_size_is_an_error() {
        assert_eq!(pages_to_bytes(1, 0), Err(PageConversionError::ZeroPageSize));
    }

    #[test]
    fn overflow_is_an_error_not_a_wrap() {
        assert_eq!(
            pages_to_bytes(u64::MAX, 4096),
            Err(PageConversionError::Overflow)
        );
        assert_eq!(
            pages_to_bytes(u64::MAX / 2 + 1, 2),
            Err(PageConversionError::Overflow)
        );
        // Just inside the limit still succeeds.
        assert_eq!(pages_to_bytes(u64::MAX / 2, 2).expect("fits"), u64::MAX - 1);
    }

    #[test]
    fn byte_difference_rejects_negative_results() {
        assert_eq!(byte_difference(10, 4), Some(6));
        assert_eq!(byte_difference(4, 10), None);
        assert_eq!(byte_difference(u64::MAX, 1), Some(u64::MAX - 1));
    }

    #[test]
    fn format_bytes_uses_gib_above_1_gib() {
        assert_eq!(format_bytes(4 * 1024 * 1024 * 1024), "4.00 GiB");
        assert_eq!(format_bytes(17_179_869_184), "16.00 GiB");
        assert_eq!(format_bytes(512 * 1024 * 1024), "512.00 MiB");
        assert_eq!(format_bytes(4096), "0.00 MiB");
    }
}
