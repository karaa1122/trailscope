//! Datetime literals: `'2026-01-02'`, `'2026-01-02 10:00'`,
//! `'2026-01-02 10:00:00.123'`, or RFC 3339 with an offset. Naive values are UTC.

use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{Date, OffsetDateTime, PrimitiveDateTime};

/// Parses a datetime literal into milliseconds since the epoch.
pub fn parse_datetime_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Ok(t) = OffsetDateTime::parse(s, &Rfc3339) {
        return to_ms(t);
    }
    let normalized = s.replacen('T', " ", 1);
    let formats = [
        format_description!("[year]-[month]-[day] [hour]:[minute]:[second].[subsecond]"),
        format_description!("[year]-[month]-[day] [hour]:[minute]:[second]"),
        format_description!("[year]-[month]-[day] [hour]:[minute]"),
    ];
    for f in formats {
        if let Ok(t) = PrimitiveDateTime::parse(&normalized, &f) {
            return to_ms(t.assume_utc());
        }
    }
    let date = Date::parse(s, &format_description!("[year]-[month]-[day]")).ok()?;
    to_ms(date.midnight().assume_utc())
}

fn to_ms(t: OffsetDateTime) -> Option<i64> {
    i64::try_from(t.unix_timestamp_nanos() / 1_000_000).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        let base = 1_767_225_600_000; // 2026-01-01T00:00:00Z
        assert_eq!(parse_datetime_ms("2026-01-01"), Some(base));
        assert_eq!(parse_datetime_ms("2026-01-01 00:01"), Some(base + 60_000));
        assert_eq!(parse_datetime_ms("2026-01-01T00:00:01"), Some(base + 1_000));
        assert_eq!(
            parse_datetime_ms("2026-01-01 00:00:00.250"),
            Some(base + 250)
        );
        assert_eq!(parse_datetime_ms("2026-01-01T02:00:00+02:00"), Some(base));
        assert_eq!(parse_datetime_ms("2026-01-01T00:00:00Z"), Some(base));
        assert_eq!(parse_datetime_ms("yesterday"), None);
        assert_eq!(parse_datetime_ms("2026-13-01"), None);
    }
}
