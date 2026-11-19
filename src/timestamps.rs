use std::borrow::Cow;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

/// Provider validation and SQLite ordering share the existing RFC3339 grammar
/// and floor to epoch seconds, including before 1970. Keep the original strings
/// in snapshots; normalization here is only for validation and epoch conversion.
pub(crate) fn rfc3339_seconds(input: &str) -> Option<i64> {
    // time accepts any single-byte separator; the provider contract accepts
    // only T, t and space. Do not silently broaden validation.
    if !matches!(input.as_bytes().get(10), Some(b'T' | b't' | b' ')) {
        return None;
    }
    let mut normalized = Cow::Borrowed(input);
    // Chrono accepted :60 in any minute and timestamp() returned the preceding
    // :59's epoch second, regardless of the fraction. time restricts leap seconds
    // to month-end UTC, so preserve that existing behavior before parsing.
    if input.get(17..19) == Some("60") {
        normalized.to_mut().replace_range(17..19, "59");
    }
    let suffix = input.get(19..)?;
    let offset_start = if let Some(fraction) = suffix.strip_prefix('.') {
        20 + fraction.bytes().take_while(u8::is_ascii_digit).count()
    } else {
        19
    };
    // Preserve the formerly accepted U+2212 offset sign, but never normalize
    // Unicode punctuation in the date, time, or fractional digits.
    if input.get(offset_start..)?.starts_with('−') {
        normalized
            .to_mut()
            .replace_range(offset_start..offset_start + '−'.len_utf8(), "-");
    }
    OffsetDateTime::parse(&normalized, &Rfc3339)
        .ok()
        .map(OffsetDateTime::unix_timestamp)
}
