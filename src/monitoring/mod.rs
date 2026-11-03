//! Live data collection.
//!
//! Each sampler converts raw data from `sysinfo` and the platform layer into
//! the models in [`crate::core::models`]. The [`collector`] runs them on a
//! dedicated background thread and publishes snapshots over a channel.

pub mod collector;
pub mod cpu;
pub mod disk;
pub mod memory;
pub mod network;
pub mod processes;

/// Converts a byte delta over `elapsed_secs` into bytes per second.
pub(crate) fn per_second(delta: u64, elapsed_secs: f64) -> f64 {
    if elapsed_secs > 0.0 {
        delta as f64 / elapsed_secs
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn per_second_guards_zero_elapsed() {
        assert_eq!(super::per_second(1000, 0.0), 0.0);
        assert_eq!(super::per_second(1000, 2.0), 500.0);
    }
}
