//! Connection pool for Modbus/TCP transports.
//!
//! Two-pool model per TCP Guide §4.2.1: priority connections (configured devices,
//! never evicted) and non-priority connections (evicted oldest-first when full).

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all, clippy::pedantic)]

pub mod backoff;
pub mod config;
pub mod connection;
pub mod error;
pub mod health;
pub mod pool;

pub use config::{BackoffConfig, PoolConfig, PriorityDevice};
pub use connection::{LeaseInvalidationReason, PooledConnection};
pub use error::PoolError;
pub use pool::ConnectionPool;
