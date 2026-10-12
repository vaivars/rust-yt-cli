use chrono::{DateTime, Local, NaiveDate, SecondsFormat, Utc};

/// `2026-10-09 14:05` in the local time zone.
pub fn local(millis: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(millis)
        .map(|time| {
            time.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// `2026-10-09T12:05:00Z`, for JSON output.
pub fn utc(millis: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(millis)
        .map(|time| time.to_rfc3339_opts(SecondsFormat::Secs, true))
        .unwrap_or_default()
}

/// `2026-10-09`; YouTrack stores date-only values as midnight UTC.
pub fn date(millis: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(millis)
        .map(|time| time.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// Midnight UTC of a `YYYY-MM-DD` day in milliseconds, the form YouTrack stores dates in.
pub fn day_millis(value: &str) -> Option<i64> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .and_then(|day| day.and_hms_opt(0, 0, 0))
        .map(|midnight| midnight.and_utc().timestamp_millis())
}

/// Today in the local time zone as `YYYY-MM-DD`.
pub fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// Minutes as hours and minutes: `12h 30m`, `45m`, `0m`.
pub fn hours_minutes(minutes: i64) -> String {
    match (minutes / 60, minutes % 60) {
        (0, rest) => format!("{rest}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, rest) => format!("{hours}h {rest}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_and_dates() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(1_791_590_400_000), "2026-10-10T00:00:00Z");
        assert_eq!(date(1_791_590_400_000), "2026-10-10");
    }

    #[test]
    fn days_are_midnight_utc_and_print_back_unchanged() {
        assert_eq!(day_millis("2026-10-10"), Some(1_791_590_400_000));
        for day in ["2026-01-01", "2026-03-29", "2026-10-25", "2024-02-29"] {
            assert_eq!(date(day_millis(day).unwrap()), day);
        }
        assert_eq!(day_millis("2026-02-30"), None);
        assert_eq!(day_millis("10.10.2026"), None);
        assert_eq!(day_millis(""), None);
    }

    #[test]
    fn today_is_a_local_day() {
        assert!(day_millis(&today()).is_some());
    }

    #[test]
    fn minutes_read_as_hours_and_minutes() {
        assert_eq!(hours_minutes(0), "0m");
        assert_eq!(hours_minutes(45), "45m");
        assert_eq!(hours_minutes(120), "2h");
        assert_eq!(hours_minutes(750), "12h 30m");
    }

    #[test]
    fn local_has_minutes() {
        assert_eq!(local(1_791_590_400_000).len(), "2026-10-10 00:00".len());
    }
}
