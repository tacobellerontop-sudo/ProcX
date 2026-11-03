//! Time helpers.

use std::time::{Duration, SystemTime};

/// Describes how long ago `then` was relative to `now`, e.g. "12s ago".
pub fn relative(then: SystemTime, now: SystemTime) -> String {
    let elapsed = now.duration_since(then).unwrap_or(Duration::ZERO).as_secs();
    match elapsed {
        0..=1 => "just now".to_owned(),
        2..=59 => format!("{elapsed}s ago"),
        60..=3_599 => format!("{}m ago", elapsed / 60),
        _ => format!("{}h ago", elapsed / 3_600),
    }
}

/// Formats a timestamp as `YYYY-MM-DD HH:MM:SS UTC` without a timezone database.
pub fn format_utc(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to a (year, month, day) civil date
/// (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_utc_dates() {
        assert_eq!(format_utc(SystemTime::UNIX_EPOCH), "1970-01-01 00:00:00 UTC");
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_709_210_096); // 2024-02-29 12:34:56
        assert_eq!(format_utc(t), "2024-02-29 12:34:56 UTC");
    }

    #[test]
    fn relative_times() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        assert_eq!(relative(now, now), "just now");
        assert_eq!(relative(now - Duration::from_secs(30), now), "30s ago");
        assert_eq!(relative(now - Duration::from_secs(600), now), "10m ago");
        assert_eq!(relative(now - Duration::from_secs(7_200), now), "2h ago");
        // A timestamp in the future (clock skew) must not panic.
        assert_eq!(relative(now + Duration::from_secs(5), now), "just now");
    }
}
