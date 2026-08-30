//! Connection pool for Modbus/TCP transports.
//!
//! Two-pool model per TCP Guide §4.2.1: priority connections (configured devices,
//! never evicted) and non-priority connections (evicted oldest-first when full).
//! Enable the opt-in `client` feature to consume a raw [`PooledConnection`] as a
//! high-level client whose TCP connection is always retired after both client-owned
//! transport halves are gone. Raw leases keep their default return-to-idle behavior.

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all, clippy::pedantic)]

pub mod backoff;
#[cfg(feature = "client")]
mod client_handoff;
pub mod config;
pub mod connection;
pub mod error;
pub mod health;
pub mod pool;

pub use config::{BackoffConfig, PoolConfig, PriorityDevice};
pub use connection::{LeaseInvalidationReason, PooledConnection};
pub use error::PoolError;
pub use pool::ConnectionPool;

#[cfg(feature = "client")]
pub use rusty_modbus_client::{ClientConfig, ClientError, RetryConfig};
