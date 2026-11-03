//! Bounded in-memory time series used for live charts.

use std::collections::VecDeque;

/// A fixed-capacity series of `(seconds, value)` samples.
///
/// Old samples are discarded once capacity is reached, so memory use is
/// bounded regardless of how long the application runs.
#[derive(Debug, Clone)]
pub struct TimeSeries {
    samples: VecDeque<(f64, f64)>,
    capacity: usize,
}

impl TimeSeries {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(2);
        Self {
            samples: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, t: f64, value: f64) {
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back((t, value));
    }

    /// Changes capacity, dropping the oldest samples if needed.
    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity.max(2);
        while self.samples.len() > self.capacity {
            self.samples.pop_front();
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.samples.len()
    }

    #[cfg(test)]
    fn latest(&self) -> Option<f64> {
        self.samples.back().map(|&(_, v)| v)
    }

    /// Mean of samples whose time is within `window` seconds of the newest one.
    pub fn mean_over(&self, window: f64) -> Option<f64> {
        let &(newest, _) = self.samples.back()?;
        let (sum, n) = self
            .samples
            .iter()
            .rev()
            .take_while(|&&(t, _)| newest - t <= window)
            .fold((0.0, 0usize), |(s, n), &(_, v)| (s + v, n + 1));
        (n > 0).then(|| sum / n as f64)
    }

    /// Points with time expressed relative to `now` (so the newest is near 0).
    pub fn relative_points(&self, now: f64) -> Vec<[f64; 2]> {
        self.samples.iter().map(|&(t, v)| [t - now, v]).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_oldest_when_full() {
        let mut s = TimeSeries::new(3);
        for i in 0..5 {
            s.push(i as f64, i as f64 * 10.0);
        }
        assert_eq!(s.len(), 3);
        assert_eq!(
            s.relative_points(4.0),
            vec![[-2.0, 20.0], [-1.0, 30.0], [0.0, 40.0]]
        );
    }

    #[test]
    fn shrinking_capacity_keeps_newest() {
        let mut s = TimeSeries::new(10);
        for i in 0..10 {
            s.push(i as f64, i as f64);
        }
        s.set_capacity(4);
        assert_eq!(s.len(), 4);
        assert_eq!(s.latest(), Some(9.0));
    }

    #[test]
    fn mean_over_window() {
        let mut s = TimeSeries::new(10);
        assert_eq!(s.mean_over(5.0), None);
        s.push(0.0, 100.0);
        s.push(8.0, 10.0);
        s.push(10.0, 20.0);
        assert_eq!(s.mean_over(5.0), Some(15.0));
    }
}
