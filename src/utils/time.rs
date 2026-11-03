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

#[cfg(test)]
mod tests {
    use super::*;

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
