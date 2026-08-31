//! Pool configuration types.

use std::net::SocketAddr;
use std::time::Duration;

use rusty_modbus_tcp::TcpConfig;

/// Connection pool configuration.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Maximum connections in the **non-priority** pool. Default: 64.
    ///
    /// Priority devices have their own per-device budgets
    /// ([`PriorityDevice::max_connections`]) and do **not** count against this.
    pub max_connections: usize,
    /// Priority device entries — healthy/unknown connections to these addresses
    /// are never age- or capacity-evicted. Known-adverse idle transports are retired.
    pub priority_devices: Vec<PriorityDevice>,
    /// Pre-connect to priority devices at pool creation time. Default: `true`.
    pub pre_connect: bool,
    /// Maintain at least one idle TCP connection per distinct configured priority
    /// address whenever its per-device capacity and connectivity permit. Default:
    /// `false`.
    ///
    /// Enabling this also starts the initial priority warm-up when
    /// [`Self::pre_connect`] is `false`. The standing task uses the first matching
    /// [`PriorityDevice`] entry's capacity, reconnects with [`Self::backoff`], and
    /// uses [`Self::health_check_interval`] as a safety-only reevaluation fallback.
    /// It performs no active liveness probe.
    ///
    /// Adding this public field is source-breaking for exhaustive `PoolConfig`
    /// struct literals. Callers should set it explicitly or include
    /// `..PoolConfig::default()` so future configuration fields remain compatible.
    pub priority_replenishment: bool,
    /// Idle timeout before a non-priority connection is eligible for eviction. Default: 300s.
    pub idle_timeout: Duration,
    /// Interval for passive idle validation and non-priority age eviction. Default: 60s.
    pub health_check_interval: Duration,
    /// Priority warm-up and replenishment retry backoff configuration.
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
            priority_replenishment: false,
            idle_timeout: Duration::from_mins(5),
            health_check_interval: Duration::from_mins(1),
            backoff: BackoffConfig::default(),
            tcp_config: TcpConfig::default(),
        }
    }
}

/// A priority device entry — healthy/unknown connections to this address are never
/// age- or capacity-evicted; known-adverse idle transports are retired.
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
