//! Gateway configuration types.

use std::net::SocketAddr;
use std::ops::RangeInclusive;
use std::time::Duration;

use rusty_modbus_tcp::config::TcpServerConfig;

/// Gateway configuration.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// TCP listen address for incoming Modbus/TCP clients. Default: `0.0.0.0:502`.
    pub tcp_listen: SocketAddr,
    /// Routing table: maps unit ID ranges to RTU-over-TCP backend addresses.
    pub routes: Vec<RouteEntry>,
    /// Timeout waiting for a serial device response. Default: 1s.
    pub serial_timeout: Duration,
    /// Maximum TCP client connections. Default: 64.
    pub max_tcp_connections: usize,
    /// Underlying TCP server config.
    pub tcp_config: TcpServerConfig,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            tcp_listen: "0.0.0.0:502".parse().unwrap(),
            routes: Vec::new(),
            serial_timeout: Duration::from_secs(1),
            max_tcp_connections: 64,
            tcp_config: TcpServerConfig::default(),
        }
    }
}

/// A routing table entry mapping unit ID ranges to backend addresses.
///
/// For CI/testing, the backend is an RTU-over-TCP address. In production,
/// this would be a serial port path.
#[derive(Debug, Clone)]
pub struct RouteEntry {
    /// Inclusive range of unit IDs handled by this route.
    pub unit_id_range: RangeInclusive<u8>,
    /// Backend RTU-over-TCP address (for CI testing; serial path in production).
    pub backend_addr: SocketAddr,
}
