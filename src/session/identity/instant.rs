//! Minimal instant handling for delegation windows. The crate carries no time
//! dependency, so this hand-rolls an RFC 3339 UTC (`Z`-offset only) parser to
//! epoch milliseconds, plus `parse_event_instant`, which normalizes both
//! `occurredAt` forms the store mints — `unix-ms:<millis>` from the local clock
//! and the RFC 3339 instants adapters ingest — to one comparable instant.

/// Normalize an event `occurredAt` to epoch milliseconds. The store mints two
/// forms in practice — `unix-ms:<millis>` (the local clock) and RFC 3339 UTC
/// (adapter-ingested and signature events) — and this collapses both to one
/// comparable instant. Returns `None` for any other shape so resolution can
/// report `UnparseableTimestamp` rather than guess.
pub(crate) fn parse_event_instant(value: &str) -> Option<i64> {
    if let Some(millis) = value.strip_prefix("unix-ms:") {
        if millis.is_empty() {
            return None;
        }
        return millis.parse().ok();
    }
    parse_rfc3339_utc_millis(value)
}

/// Parse an RFC 3339 UTC instant — `YYYY-MM-DDTHH:MM:SS[.fraction]Z` — to epoch
/// milliseconds. Returns `None` for any other shape, including non-`Z` offsets,
/// a missing `T`/`Z`, out-of-range fields, or non-digit characters. Fractional
/// seconds are truncated to millisecond precision.
pub(crate) fn parse_rfc3339_utc_millis(value: &str) -> Option<i64> {
    let body = value.strip_suffix('Z')?;
    let (date, time) = body.split_once('T')?;

    let mut date_parts = date.split('-');
    let year: i64 = parse_fixed_digits(date_parts.next()?, 4)?;
    let month: i64 = parse_fixed_digits(date_parts.next()?, 2)?;
    let day: i64 = parse_fixed_digits(date_parts.next()?, 2)?;
    if date_parts.next().is_some() {
        return None;
    }
    if !(1..=12).contains(&month) || !(1..=days_in_month(year, month)?).contains(&day) {
        return None;
    }

    let (clock, fraction_millis) = match time.split_once('.') {
        Some((clock, fraction)) => (clock, parse_fraction_millis(fraction)?),
        None => (time, 0),
    };
    let mut clock_parts = clock.split(':');
    let hour: i64 = parse_fixed_digits(clock_parts.next()?, 2)?;
    let minute: i64 = parse_fixed_digits(clock_parts.next()?, 2)?;
    let second: i64 = parse_fixed_digits(clock_parts.next()?, 2)?;
    if clock_parts.next().is_some() {
        return None;
    }
    // Allow second == 60 for a leap second; clamp nothing else.
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let days = days_from_civil(year, month, day);
    let seconds = ((days * 24 + hour) * 60 + minute) * 60 + second;
    Some(seconds * 1_000 + fraction_millis)
}

/// Parse exactly `width` ASCII digits to a non-negative integer.
fn parse_fixed_digits(value: &str, width: usize) -> Option<i64> {
    if value.len() != width || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

/// Truncate a fractional-seconds string to milliseconds. The fraction must be
/// all digits and non-empty.
fn parse_fraction_millis(fraction: &str) -> Option<i64> {
    if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut millis_digits = String::from("000");
    millis_digits.replace_range(..fraction.len().min(3), &fraction[..fraction.len().min(3)]);
    millis_digits.parse().ok()
}

/// Days in `month` of `year`, honoring leap years. `None` for an out-of-range
/// month.
fn days_in_month(year: i64, month: i64) -> Option<i64> {
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => return None,
    };
    Some(days)
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Days since the Unix epoch (1970-01-01) for a proleptic-Gregorian date, after
/// Howard Hinnant's `days_from_civil`.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_epoch_and_known_instants() {
        assert_eq!(parse_rfc3339_utc_millis("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339_utc_millis("2026-06-10T00:00:00Z"),
            Some(1_781_049_600_000)
        );
    }

    #[test]
    fn truncates_fractional_seconds_to_millis() {
        assert_eq!(
            parse_rfc3339_utc_millis("1970-01-01T00:00:00.250Z"),
            Some(250)
        );
        assert_eq!(
            parse_rfc3339_utc_millis("1970-01-01T00:00:00.123456Z"),
            Some(123)
        );
        assert_eq!(
            parse_rfc3339_utc_millis("1970-01-01T00:00:00.5Z"),
            Some(500)
        );
    }

    #[test]
    fn rejects_non_utc_and_malformed_shapes() {
        for bad in [
            "2026-06-10T00:00:00+02:00",
            "2026-06-10T00:00:00",
            "2026-06-10 00:00:00Z",
            "2026-13-01T00:00:00Z",
            "2026-06-31T00:00:00Z",
            "2026-06-10T24:00:00Z",
            "2026-06-10T00:60:00Z",
            "yesterday",
            "2026-6-10T00:00:00Z",
            "2026-06-10T00:00:00.Z",
        ] {
            assert_eq!(parse_rfc3339_utc_millis(bad), None, "{bad} must not parse");
        }
    }

    #[test]
    fn honors_leap_years() {
        assert!(parse_rfc3339_utc_millis("2024-02-29T00:00:00Z").is_some());
        assert_eq!(parse_rfc3339_utc_millis("2026-02-29T00:00:00Z"), None);
    }

    #[test]
    fn event_instant_normalizes_both_forms() {
        // RFC 3339 UTC and its unix-ms: equivalent normalize to the same epoch.
        let rfc3339 = parse_event_instant("2026-06-11T00:00:00Z").unwrap();
        let unix_ms = parse_event_instant("unix-ms:1781136000000").unwrap();
        assert_eq!(rfc3339, unix_ms);
        assert_eq!(unix_ms, 1_781_136_000_000);
    }

    #[test]
    fn event_instant_rejects_unparseable_and_bad_unix_ms() {
        assert_eq!(parse_event_instant("garbage"), None);
        assert_eq!(parse_event_instant("unix-ms:"), None);
        assert_eq!(parse_event_instant("unix-ms:not-a-number"), None);
        assert_eq!(parse_event_instant("2026-06-11T00:00:00+02:00"), None);
    }
}
