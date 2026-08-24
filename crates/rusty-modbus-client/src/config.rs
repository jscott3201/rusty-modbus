//! Client configuration types.

use std::time::Duration;

use rusty_modbus_types::{ExceptionCode, UnitId};

/// Client configuration.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Default unit ID for requests. Default: `UnitId(0xFF)` (direct TCP device).
    pub unit_id: UnitId,
    /// Per-attempt timeout after admission. The operation envelope starts when
    /// an admission permit is acquired; waiting for that permit is not timed.
    /// Default: 5s.
    pub timeout: Duration,
    /// Maximum concurrent admitted logical operations. A permit is held across
    /// retries and backoff. Default: 16.
    pub max_in_flight: usize,
    /// Retry configuration.
    pub retry: RetryConfig,
    /// Time allowed for admitted logical operations to drain after shutdown
    /// seals admission. The client cancels remaining work and joins its tasks
    /// when this duration expires. Default: 10s.
    pub shutdown_timeout: Duration,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            unit_id: UnitId(0xFF),
            timeout: Duration::from_secs(5),
            max_in_flight: 16,
            retry: RetryConfig::default(),
            shutdown_timeout: Duration::from_secs(10),
        }
    }
}

/// Retry configuration for transient failures.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts. Default: 3.
    pub max_retries: u32,
    /// Delay between retry attempts. Default: 100ms.
    pub retry_delay: Duration,
    /// Exception codes selected for retry. Default: `[ServerDeviceBusy]`.
    ///
    /// Only `ServerDeviceBusy` is eligible for exception-driven replay.
    /// `Acknowledge` remains terminal even when included here.
    pub retryable_exceptions: Vec<ExceptionCode>,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            retry_delay: Duration::from_millis(100),
            retryable_exceptions: vec![ExceptionCode::ServerDeviceBusy],
        }
    }
}

impl RetryConfig {
    /// Check whether an exception code is effectively eligible for retry.
    #[must_use]
    pub fn is_retryable(&self, code: ExceptionCode) -> bool {
        code == ExceptionCode::ServerDeviceBusy
            && self
                .retryable_exceptions
                .contains(&ExceptionCode::ServerDeviceBusy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_retry_exceptions_include_busy_but_not_acknowledge() {
        let retry = RetryConfig::default();
        assert!(retry.is_retryable(ExceptionCode::ServerDeviceBusy));
        assert!(!retry.is_retryable(ExceptionCode::Acknowledge));
    }

    #[test]
    fn effective_retry_eligibility_rejects_configured_non_busy_exceptions() {
        let retry = RetryConfig {
            retryable_exceptions: vec![ExceptionCode::Acknowledge, ExceptionCode::IllegalDataValue],
            ..RetryConfig::default()
        };

        assert!(!retry.is_retryable(ExceptionCode::Acknowledge));
        assert!(!retry.is_retryable(ExceptionCode::IllegalDataValue));
        assert!(!retry.is_retryable(ExceptionCode::ServerDeviceBusy));
    }
}
