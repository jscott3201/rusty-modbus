//! Server configuration types.

use std::net::SocketAddr;
use std::time::Duration;

use rusty_modbus_tcp::config::TcpServerConfig;
use rusty_modbus_types::UnitId;

use crate::error::ServerConfigError;

/// Server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Address to listen on.
    pub listen_addr: SocketAddr,
    /// This server's Unit ID. Default: `UnitId(1)`.
    pub unit_id: UnitId,
    /// Maximum concurrent connections. Default: 64.
    pub max_connections: usize,
    /// Maximum concurrent transactions per connection. Default: 16.
    pub max_transactions: u16,
    /// Shutdown timeout. Default: 10s.
    pub shutdown_timeout: Duration,
    /// Device identification for FC 0x2B/0x0E.
    pub device_id: DeviceIdentification,
    /// Underlying TCP server config.
    pub tcp_config: TcpServerConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:502".parse().unwrap(),
            unit_id: UnitId(1),
            max_connections: 64,
            max_transactions: 16,
            shutdown_timeout: Duration::from_secs(10),
            device_id: DeviceIdentification::default(),
            tcp_config: TcpServerConfig::default(),
        }
    }
}

impl ServerConfig {
    /// Validate limits that must be usable before the listen socket is bound.
    ///
    /// `max_transactions` is validated as configuration only. The current
    /// server processes one request at a time per connection and does not
    /// enforce this value at runtime.
    ///
    /// # Errors
    ///
    /// Returns the first zero connection, transaction, or shutdown limit.
    pub fn validate(&self) -> Result<(), ServerConfigError> {
        if self.max_connections == 0 {
            return Err(ServerConfigError::ZeroMaxConnections);
        }
        if self.max_transactions == 0 {
            return Err(ServerConfigError::ZeroMaxTransactions);
        }
        if self.shutdown_timeout.is_zero() {
            return Err(ServerConfigError::ZeroShutdownTimeout);
        }
        Ok(())
    }
}

/// Device identification for Read Device ID (FC 0x2B/0x0E).
#[derive(Debug, Clone)]
pub struct DeviceIdentification {
    /// Vendor name (mandatory, object 0x00).
    pub vendor_name: String,
    /// Product code (mandatory, object 0x01).
    pub product_code: String,
    /// Major/minor revision (mandatory, object 0x02).
    pub major_minor_revision: String,
    /// Vendor URL (optional).
    pub vendor_url: Option<String>,
    /// Product name (optional).
    pub product_name: Option<String>,
    /// Model name (optional).
    pub model_name: Option<String>,
    /// User application name (optional).
    pub user_application_name: Option<String>,
}

impl Default for DeviceIdentification {
    fn default() -> Self {
        Self {
            vendor_name: String::from("rusty-modbus"),
            product_code: String::from("RMOD"),
            major_minor_revision: String::from(env!("CARGO_PKG_VERSION")),
            vendor_url: None,
            product_name: None,
            model_name: None,
            user_application_name: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{DeviceIdentification, ServerConfig};
    use crate::error::ServerConfigError;

    #[test]
    fn default_device_revision_matches_package_version() {
        let device_id = DeviceIdentification::default();
        assert_eq!(device_id.major_minor_revision, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn configured_device_revision_is_preserved() {
        let device_id = DeviceIdentification {
            major_minor_revision: String::from("device-firmware-7"),
            ..DeviceIdentification::default()
        };
        assert_eq!(device_id.major_minor_revision, "device-firmware-7");
    }

    #[test]
    fn zero_server_limits_are_rejected() {
        let config = ServerConfig {
            max_connections: 0,
            ..ServerConfig::default()
        };
        assert_eq!(
            config.validate(),
            Err(ServerConfigError::ZeroMaxConnections)
        );

        let config = ServerConfig {
            max_transactions: 0,
            ..ServerConfig::default()
        };
        assert_eq!(
            config.validate(),
            Err(ServerConfigError::ZeroMaxTransactions)
        );

        let config = ServerConfig {
            shutdown_timeout: Duration::ZERO,
            ..ServerConfig::default()
        };
        assert_eq!(
            config.validate(),
            Err(ServerConfigError::ZeroShutdownTimeout)
        );
    }

    #[test]
    fn transaction_limit_above_client_ring_size_is_valid_configuration() {
        let config = ServerConfig {
            max_transactions: 17,
            ..ServerConfig::default()
        };
        assert_eq!(config.validate(), Ok(()));
    }
}
