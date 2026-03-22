//! `rusty-modbus` — Complete Modbus protocol stack for Rust.
//!
//! Unified facade crate that re-exports the modular subcrates behind feature flags.
//! Add `rusty-modbus` as your single dependency and enable the features you need.
//!
//! # Feature Flags
//!
//! | Feature   | Default | Description |
//! |-----------|---------|-------------|
//! | `tcp`     | yes     | TCP transport + client |
//! | `rtu`     | no      | RTU transport (serial + RTU-over-TCP) |
//! | `rtu-tcp` | no      | Alias for `rtu` |
//! | `server`  | no      | Modbus server with pluggable data store |
//! | `gateway` | no      | TCP ↔ RTU bridge gateway |
//! | `tls`     | no      | TLS 1.3 transport (Modbus/TCP Security V36) |
//! | `pool`    | no      | Connection pooling |
//! | `full`    | no      | All features above |

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all, clippy::pedantic)]

// Always available — foundation crates.
pub use modbus_types as types;
pub use modbus_codec as codec;
pub use modbus_frame as frame;

// TCP transport + client (default feature).
#[cfg(feature = "tcp")]
pub use modbus_tcp as tcp;

#[cfg(feature = "tcp")]
pub use modbus_client as client;

/// Convenience alias for [`modbus_client::ModbusClient`].
#[cfg(feature = "tcp")]
pub type Client = modbus_client::ModbusClient;

// RTU transport.
#[cfg(feature = "rtu")]
pub use modbus_rtu as rtu;

// TLS transport.
#[cfg(feature = "tls")]
pub use modbus_tls as tls;

// Connection pool.
#[cfg(feature = "pool")]
pub use modbus_pool as pool;

// Server.
#[cfg(feature = "server")]
pub use modbus_server as server;

/// Convenience alias for [`modbus_server::ModbusServer`].
#[cfg(feature = "server")]
pub type Server<S> = modbus_server::ModbusServer<S>;

// Gateway.
#[cfg(feature = "gateway")]
pub use modbus_gateway as gateway;

/// Convenience alias for [`modbus_gateway::ModbusGateway`].
#[cfg(feature = "gateway")]
pub type Gateway = modbus_gateway::ModbusGateway;
