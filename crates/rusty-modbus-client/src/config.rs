//! Client configuration types.

use std::time::Duration;

use rusty_modbus_types::{ExceptionCode, UnitId};

/// Client configuration.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Default unit ID for requests. Default: `UnitId(0xFF)` (direct TCP device).
    pub unit_id: UnitId,
    /// Per-request timeout. Default: 5s.
    pub timeout: Duration,
    /// Maximum concurrent in-flight transactions. Default: 16 (spec max).
    pub max_in_flight: usize,
    /// Retry configuration.
    pub retry: RetryConfig,
    /// Shutdown timeout — how long to wait for in-flight requests during shutdown. Default: 10s.
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
    /// Exception codes that trigger a retry. Default: `[ServerDeviceBusy, Acknowledge]`.
    pub retryable_exceptions: Vec<ExceptionCode>,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            retry_delay: Duration::from_millis(100),
            retryable_exceptions: vec![ExceptionCode::ServerDeviceBusy, ExceptionCode::Acknowledge],
        }
    }
}

impl RetryConfig {
    /// Check if an exception code should trigger a retry.
    #[must_use]
    pub fn is_retryable(&self, code: ExceptionCode) -> bool {
        self.retryable_exceptions.contains(&code)
    }
}
