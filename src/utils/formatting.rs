//! Human-readable formatting of metrics.

use std::time::Duration;

const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];

/// Formats a byte count using binary (1024-based) units, as Task Manager does.
pub fn bytes(value: u64) -> String {
    bytes_f64(value as f64)
}

fn bytes_f64(mut value: f64) -> String {
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 || value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Formats a throughput in bytes per second.
pub fn rate(bytes_per_sec: f64) -> String {
    format!("{}/s", bytes_f64(bytes_per_sec.max(0.0)))
}

pub fn percent(value: f32) -> String {
    format!("{value:.1}%")
}

/// Marker for values the platform could not provide.
pub const UNAVAILABLE: &str = "—";

/// Inserts thousands separators: 1234567 -> "1,234,567".
pub fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Formats a link speed in decimal bits per second (as network gear does).
pub fn bits_per_sec(bps: u64) -> String {
    match bps {
        b if b >= 1_000_000_000 => format!("{:.1} Gbps", b as f64 / 1e9),
        b if b >= 1_000_000 => format!("{:.0} Mbps", b as f64 / 1e6),
        b if b >= 1_000 => format!("{:.0} Kbps", b as f64 / 1e3),
        b => format!("{b} bps"),
    }
}

/// Formats a duration as `3d 04:05:06` or `04:05:06`.
pub fn uptime(duration: Duration) -> String {
    let total = duration.as_secs();
    let (days, rem) = (total / 86_400, total % 86_400);
    let (h, m, s) = (rem / 3_600, (rem % 3_600) / 60, rem % 60);
    if days > 0 {
        format!("{days}d {h:02}:{m:02}:{s:02}")
    } else {
        format!("{h:02}:{m:02}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1536), "1.5 KB");
        assert_eq!(bytes(16 * 1024 * 1024 * 1024), "16.0 GB");
        assert_eq!(bytes(512 * 1024 * 1024), "512 MB");
    }

    #[test]
    fn formats_rates_and_counts() {
        assert_eq!(rate(-5.0), "0 B/s");
        assert_eq!(rate(2048.0), "2.0 KB/s");
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(thousands(999), "999");
        assert_eq!(bits_per_sec(1_000_000_000), "1.0 Gbps");
        assert_eq!(bits_per_sec(100_000_000), "100 Mbps");
        assert_eq!(thousands(1000), "1,000");
    }

    #[test]
    fn formats_uptime() {
        assert_eq!(uptime(Duration::from_secs(59)), "00:00:59");
        assert_eq!(uptime(Duration::from_secs(90_061)), "1d 01:01:01");
    }
}
