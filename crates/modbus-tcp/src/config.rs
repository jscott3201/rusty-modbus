//! TCP transport configuration.

use std::net::IpAddr;
use std::time::Duration;

/// TCP client/connection configuration.
///
/// Defaults follow the Modbus TCP Implementation Guide §4.2–4.3.
#[derive(Debug, Clone)]
pub struct TcpConfig {
    /// TCP connection timeout. Default: 5s (spec notes 75s Berkeley default is too long).
    pub connect_timeout: Duration,
    /// Read timeout for receiving frames. Default: 30s. `None` = no timeout.
    pub read_timeout: Option<Duration>,
    /// Write timeout for sending frames. Default: 30s. `None` = no timeout.
    pub write_timeout: Option<Duration>,
    /// Disable Nagle's algorithm. Default: `true` (spec §4.3.2 — recommended for real-time).
    pub tcp_nodelay: bool,
    /// TCP keepalive idle time. Default: `Some(60s)`. `None` = disabled.
    pub keepalive: Option<Duration>,
    /// Target port. Default: 502 (IANA registered for Modbus/TCP).
    pub port: u16,
}

impl Default for TcpConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            read_timeout: Some(Duration::from_secs(30)),
            write_timeout: Some(Duration::from_secs(30)),
            tcp_nodelay: true,
            keepalive: Some(Duration::from_secs(60)),
            port: 502,
        }
    }
}

/// TCP server listener configuration.
#[derive(Debug, Clone)]
pub struct TcpServerConfig {
    /// Base TCP settings.
    pub tcp: TcpConfig,
    /// Maximum simultaneous connections. Default: 64.
    pub max_connections: usize,
    /// IP-based access control. `None` = accept all.
    pub access_control: Option<AccessControl>,
}

impl Default for TcpServerConfig {
    fn default() -> Self {
        Self {
            tcp: TcpConfig::default(),
            max_connections: 64,
            access_control: None,
        }
    }
}

/// IP-based access control for the server listener (TCP Guide §4.2.3).
#[derive(Debug, Clone)]
pub struct AccessControl {
    /// Default policy for IPs not in the rules list.
    pub default_mode: AccessMode,
    /// Per-IP overrides.
    pub rules: Vec<(IpAddr, AccessMode)>,
}

impl AccessControl {
    /// Check whether the given IP address is allowed.
    #[must_use]
    pub fn is_allowed(&self, ip: &IpAddr) -> bool {
        for (rule_ip, mode) in &self.rules {
            if rule_ip == ip {
                return matches!(mode, AccessMode::Allow);
            }
        }
        matches!(self.default_mode, AccessMode::Allow)
    }
}

/// Access control policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    /// Allow connections from this IP.
    Allow,
    /// Deny connections from this IP.
    Deny,
}
