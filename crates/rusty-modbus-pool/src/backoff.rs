//! Exponential backoff for reconnection attempts.

use std::time::Duration;

use crate::config::BackoffConfig;

/// Exponential backoff state tracker.
#[derive(Debug)]
pub struct Backoff {
    config: BackoffConfig,
    current_delay: Duration,
}

impl Backoff {
    /// Create a new backoff tracker from the given configuration.
    #[must_use]
    pub fn new(config: BackoffConfig) -> Self {
        let current_delay = config.initial_delay;
        Self {
            config,
            current_delay,
        }
    }

    /// Returns the current delay, then increases it for the next call.
    pub fn next_delay(&mut self) -> Duration {
        let delay = self.current_delay;
        self.current_delay = self
            .current_delay
            .mul_f64(self.config.multiplier)
            .min(self.config.max_delay);
        delay
    }

    /// Reset the backoff to its initial delay (e.g., after a successful connection).
    pub fn reset(&mut self) {
        self.current_delay = self.config.initial_delay;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_increases_exponentially() {
        let mut b = Backoff::new(BackoffConfig {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            multiplier: 2.0,
        });

        assert_eq!(b.next_delay(), Duration::from_millis(100));
        assert_eq!(b.next_delay(), Duration::from_millis(200));
        assert_eq!(b.next_delay(), Duration::from_millis(400));
        assert_eq!(b.next_delay(), Duration::from_millis(800));
    }

    #[test]
    fn backoff_caps_at_max() {
        let mut b = Backoff::new(BackoffConfig {
            initial_delay: Duration::from_secs(5),
            max_delay: Duration::from_secs(10),
            multiplier: 3.0,
        });

        assert_eq!(b.next_delay(), Duration::from_secs(5));
        assert_eq!(b.next_delay(), Duration::from_secs(10)); // 15 capped to 10
        assert_eq!(b.next_delay(), Duration::from_secs(10)); // stays at max
    }

    #[test]
    fn backoff_resets() {
        let mut b = Backoff::new(BackoffConfig::default());
        b.next_delay();
        b.next_delay();
        b.reset();
        assert_eq!(b.next_delay(), Duration::from_millis(100));
    }
}
