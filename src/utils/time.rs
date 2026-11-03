//! Time helpers.

use std::sync::atomic::{AtomicI32, Ordering};
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

/// Local UTC offset in seconds, published by the UI from collector samples
/// (the platform query runs on the collector thread). `i32::MIN` = unknown.
static UTC_OFFSET: AtomicI32 = AtomicI32::new(i32::MIN);

pub fn set_utc_offset(offset_secs: Option<i32>) {
    UTC_OFFSET.store(offset_secs.unwrap_or(i32::MIN), Ordering::Relaxed);
}

/// The local offset, or `None` when unknown (times are then shown in UTC).
pub fn utc_offset() -> Option<i32> {
    let v = UTC_OFFSET.load(Ordering::Relaxed);
    (v != i32::MIN).then_some(v)
}

/// `" UTC"` when local time is unknown, so every displayed time says which
/// zone it is in; empty when times are local.
pub fn zone_suffix() -> &'static str {
    if utc_offset().is_some() { "" } else { " UTC" }
}

fn unix_secs(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn date_time(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Formats a timestamp as `YYYY-MM-DD HH:MM:SS UTC` without a timezone database.
pub fn format_utc(t: SystemTime) -> String {
    format!("{} UTC", date_time(unix_secs(t)))
}

/// Formats a timestamp in local time when the offset is known, else in UTC
/// (labelled).
pub fn format_local(t: SystemTime) -> String {
    match utc_offset() {
        Some(offset) => date_time(unix_secs(t) + i64::from(offset)),
        None => format_utc(t),
    }
}

/// Formats Unix seconds as `HH:MM` in local time (UTC when unknown),
/// prefixed with `MM-DD ` when `with_date` is set. Used for chart axes.
pub fn format_clock(unix_secs: i64, with_date: bool) -> String {
    format_clock_at(unix_secs, with_date, utc_offset().unwrap_or(0))
}

fn format_clock_at(unix_secs: i64, with_date: bool, offset_secs: i32) -> String {
    let local = unix_secs + i64::from(offset_secs);
    let (days, rem) = (local.div_euclid(86_400), local.rem_euclid(86_400));
    let clock = format!("{:02}:{:02}", rem / 3600, (rem % 3600) / 60);
    if with_date {
        let (_, m, d) = civil_from_days(days);
        format!("{m:02}-{d:02} {clock}")
    } else {
        clock
    }
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
    fn clock_format() {
        assert_eq!(format_clock_at(1_700_000_000, false, 0), "22:13");
        assert_eq!(format_clock_at(1_700_000_000, true, 0), "11-14 22:13");
        // UTC+2 crosses midnight into the next day.
        assert_eq!(format_clock_at(1_700_000_000, true, 7_200), "11-15 00:13");
        assert_eq!(format_clock_at(1_700_000_000, false, -5 * 3600), "17:13");
    }

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
