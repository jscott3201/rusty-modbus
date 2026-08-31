//! Connection pool for Modbus/TCP transports.
//!
//! Two-pool model per TCP Guide §4.2.1: priority connections (configured devices,
//! never age- or capacity-evicted) and non-priority connections (evicted
//! oldest-first when full). Idle connections in either pool are passively retired
//! when buffered input, peer EOF, or a socket error is immediately observable.
//! Raw [`PooledConnection`] leases always retire on drop. Enable the opt-in
//! `client` feature and hand a pristine lease to [`PooledClientSession`] for the
//! only checked-out-lease path that can return a connection to idle. The same
//! feature enables explicit, default-off FC01-FC04 probes for configured
//! priority devices.

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
pub use client_handoff::{PooledClientReturnOutcome, PooledClientSession};

#[cfg(feature = "client")]
pub use error::ReusableClientHandoffError;

#[cfg(feature = "client")]
pub use config::{PriorityProbeConfig, PriorityProbeOperation};

#[cfg(feature = "client")]
pub use error::PriorityProbeConfigError;

#[cfg(feature = "client")]
pub use rusty_modbus_client::{ClientConfig, ClientError, RetryConfig};
