//! Pool configuration types.

use std::net::SocketAddr;
use std::time::Duration;

use rusty_modbus_tcp::TcpConfig;

/// Connection pool configuration.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Maximum total connections across both pools. Default: 64.
    pub max_connections: usize,
    /// Priority device entries — connections to these addresses are never evicted.
    pub priority_devices: Vec<PriorityDevice>,
    /// Pre-connect to priority devices at pool creation time. Default: `true`.
    pub pre_connect: bool,
    /// Idle timeout before a non-priority connection is eligible for eviction. Default: 300s.
    pub idle_timeout: Duration,
    /// Health check interval for probing idle connections. Default: 60s.
    pub health_check_interval: Duration,
    /// Reconnect backoff configuration.
    pub backoff: BackoffConfig,
    /// Underlying TCP transport configuration.
    pub tcp_config: TcpConfig,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 64,
            priority_devices: Vec::new(),
            pre_connect: true,
            idle_timeout: Duration::from_secs(300),
            health_check_interval: Duration::from_secs(60),
            backoff: BackoffConfig::default(),
            tcp_config: TcpConfig::default(),
        }
    }
}

/// A priority device entry — connections to this address are never locally evicted.
#[derive(Debug, Clone)]
pub struct PriorityDevice {
    /// Device socket address.
    pub addr: SocketAddr,
    /// Maximum connections to this device.
    pub max_connections: usize,
}

/// Exponential backoff configuration for reconnection attempts.
#[derive(Debug, Clone)]
pub struct BackoffConfig {
    /// Initial retry delay. Default: 100ms.
    pub initial_delay: Duration,
    /// Maximum retry delay. Default: 30s.
    pub max_delay: Duration,
    /// Multiplier applied on each retry. Default: 2.0.
    pub multiplier: f64,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            multiplier: 2.0,
        }
    }
}
