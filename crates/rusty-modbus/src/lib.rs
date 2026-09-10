//! `rusty-modbus` — Modular Modbus protocol stack for Rust.
//!
//! Unified facade crate that re-exports the modular subcrates behind feature flags.
//! Add `rusty-modbus` as your single dependency and enable the features you need.
//!
//! # Feature Flags
//!
//! | Feature   | Default | Description |
//! |-----------|---------|-------------|
//! | `tcp`     | yes     | TCP transport + client |
//! | `rtu`     | no      | RTU configuration, framing integration, and RTU-over-TCP transport |
//! | `rtu-serial` | no   | Physical serial transport in addition to `rtu` |
//! | `rtu-tcp` | no      | Alias for `rtu` without physical serial dependencies |
//! | `server`  | no      | Modbus server with pluggable data store |
//! | `gateway` | no      | TCP ↔ RTU bridge gateway |
//! | `tls`     | no      | TLS 1.3 transport; with `server`, opt-in identity-only mTLS serving |
//! | `pool`    | no      | Connection pooling with raw-drop retirement + verdict-gated TCP client reuse |
//! | `full`    | no      | All features above |

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all, clippy::pedantic)]

// Always available — foundation crates.
pub use rusty_modbus_codec as codec;
pub use rusty_modbus_frame as frame;
pub use rusty_modbus_types as types;

// TCP transport + client (default feature).
#[cfg(feature = "tcp")]
pub use rusty_modbus_tcp as tcp;

#[cfg(feature = "tcp")]
pub use rusty_modbus_client as client;

/// Convenience alias for [`rusty_modbus_client::ModbusClient`].
#[cfg(feature = "tcp")]
pub type Client = rusty_modbus_client::ModbusClient;

// RTU transport.
#[cfg(feature = "rtu")]
pub use rusty_modbus_rtu as rtu;

// TLS transport.
#[cfg(feature = "tls")]
pub use rusty_modbus_tls as tls;

// Connection pool.
#[cfg(feature = "pool")]
pub use rusty_modbus_pool as pool;

// Server.
#[cfg(feature = "server")]
pub use rusty_modbus_server as server;

/// Convenience alias for [`rusty_modbus_server::ModbusServer`].
#[cfg(feature = "server")]
pub type Server<S> = rusty_modbus_server::ModbusServer<S>;

/// Identity-only sequential mTLS server (requires both `server` and `tls`).
/// All CA-authenticated peers have datastore access; this is not role authorization.
///
/// ```no_run
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// use std::sync::Arc;
/// use rusty_modbus::{server::{InMemoryStore, StoreConfig, TlsModbusServerConfig}, tls::TlsServerConfig, TlsServer};
/// let mut config = TlsModbusServerConfig::new(TlsServerConfig {
///     server_cert: "server.pem".into(), server_key: "server-key.pem".into(),
///     ca_cert: "client-ca.pem".into(), ..TlsServerConfig::default()
/// });
/// config.listen_addr = "127.0.0.1:802".parse()?;
/// let server = TlsServer::start(config, Arc::new(InMemoryStore::new(StoreConfig::default()))).await?;
/// let _outcome = server.stop().await;
/// # Ok(()) }
/// ```
#[cfg(all(feature = "server", feature = "tls"))]
pub type TlsServer<S> = rusty_modbus_server::TlsModbusServer<S>;

// Gateway.
#[cfg(feature = "gateway")]
pub use rusty_modbus_gateway as gateway;

/// Convenience alias for [`rusty_modbus_gateway::ModbusGateway`].
#[cfg(feature = "gateway")]
pub type Gateway = rusty_modbus_gateway::ModbusGateway;
