//! Minimal instant handling for delegation windows. The crate carries no time
//! dependency, so this hand-rolls an RFC 3339 UTC (`Z`-offset only) parser to
//! epoch milliseconds, plus `parse_event_instant`, which normalizes both
//! legal `occurredAt` forms — legacy `unix-ms:<millis>` values and RFC 3339 UTC
//! values — to one comparable instant.

use std::cmp::Ordering;

/// Normalize an event `occurredAt` to epoch milliseconds. Two legal forms exist
/// in practice — legacy `unix-ms:<millis>` values and RFC 3339 UTC — and this
/// collapses both to one comparable instant. Returns `None` for any other shape
/// so resolution can report `UnparseableTimestamp` rather than guess.
pub fn parse_event_instant(value: &str) -> Option<i64> {
    if let Some(millis) = value.strip_prefix("unix-ms:") {
        if millis.is_empty() {
            return None;
        }
        return millis.parse().ok();
    }
    parse_rfc3339_utc_millis(value)
}

/// Compare event instants under one total order. Legal timestamp forms compare
/// by normalized epoch milliseconds; malformed values compare lexically before
/// legal instants. Equivalent legal forms compare equal so callers can apply
/// their domain-specific tie-break or retain the first value.
pub fn compare_event_instants(left: &str, right: &str) -> Ordering {
    match (parse_event_instant(left), parse_event_instant(right)) {
        (Some(left), Some(right)) => left.cmp(&right),
        (None, None) => left.cmp(right),
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
    }
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

/// Format epoch milliseconds as an RFC 3339 UTC instant
/// `YYYY-MM-DDTHH:MM:SS.mmmZ`. The crate carries no time dependency, so this
/// hand-rolls the civil-date math symmetric to the parser above.
pub fn format_rfc3339_utc_millis(millis: i64) -> String {
    let total_seconds = millis.div_euclid(1000);
    let millis_of_second = millis.rem_euclid(1000);
    let days = total_seconds.div_euclid(86_400);
    let secs_of_day = total_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3_600;
    let minute = (secs_of_day % 3_600) / 60;
    let second = secs_of_day % 60;
    format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis_of_second:03}Z"
    )
}

/// Inverse of `days_from_civil`: epoch-day count → (year, month, day). Hinnant's
/// algorithm, valid for the proleptic Gregorian calendar.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
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
    fn formats_epoch_millis_as_rfc3339_utc_with_millisecond_precision() {
        let millis = parse_rfc3339_utc_millis("2026-06-10T00:00:00.123Z").unwrap();
        assert_eq!(
            format_rfc3339_utc_millis(millis),
            "2026-06-10T00:00:00.123Z"
        );
        assert_eq!(format_rfc3339_utc_millis(0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn format_then_parse_round_trips_at_millisecond_precision() {
        for millis in [
            0,
            parse_rfc3339_utc_millis("2000-02-29T12:34:56.001Z").unwrap(),
            parse_rfc3339_utc_millis("2026-06-18T06:09:41.123Z").unwrap(),
            parse_rfc3339_utc_millis("2099-12-31T23:59:59.999Z").unwrap(),
            -1,
        ] {
            assert_eq!(
                parse_rfc3339_utc_millis(&format_rfc3339_utc_millis(millis)),
                Some(millis)
            );
        }
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

    #[test]
    fn event_instant_comparison_is_total_for_mixed_and_malformed_values() {
        use std::cmp::Ordering;

        let malformed = "malformed";
        let unix_ms = "unix-ms:1000";
        let rfc3339 = "1970-01-01T00:00:02Z";

        assert_eq!(compare_event_instants(malformed, unix_ms), Ordering::Less);
        assert_eq!(compare_event_instants(unix_ms, rfc3339), Ordering::Less);
        assert_eq!(compare_event_instants(malformed, rfc3339), Ordering::Less);
        assert_eq!(
            compare_event_instants("unix-ms:2000", rfc3339),
            Ordering::Equal
        );
    }
}
