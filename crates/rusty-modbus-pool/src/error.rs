//! Pool error types.

use rusty_modbus_tcp::TransportError;

#[cfg(feature = "client")]
use crate::config::PriorityProbeOperation;

/// Errors that can occur during connection pool operations.
#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    /// No connections available and pool is at capacity.
    #[error("pool exhausted: no connections available")]
    Exhausted,

    /// Failed to establish a connection.
    #[error("connection failed: {0}")]
    ConnectionFailed(#[from] TransportError),

    /// The fixed pool-capacity acquisition deadline elapsed.
    #[error("pool capacity acquisition timed out")]
    Timeout,

    /// Pool is shutting down; no new connections will be issued.
    #[error("pool is shutting down")]
    ShuttingDown,
}

/// Validation errors for read-only priority-device probes.
#[cfg(feature = "client")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PriorityProbeConfigError {
    /// Unit ID is broadcast or reserved rather than a supported read target.
    #[error("priority probe unit ID {0} is not in 1..=247 or 255")]
    InvalidUnitId(u8),
    /// Quantity is zero or exceeds the selected function's protocol maximum.
    #[error("priority probe {operation} quantity {quantity} is not in 1..={maximum}")]
    InvalidQuantity {
        /// Selected read operation.
        operation: PriorityProbeOperation,
        /// Rejected quantity.
        quantity: u16,
        /// Maximum supported quantity for this operation.
        maximum: u16,
    },
    /// The requested address span extends beyond the Modbus address space.
    #[error("priority probe address {address} plus quantity {quantity} exceeds 65536")]
    AddressSpanExceeded {
        /// First requested address.
        address: u16,
        /// Requested item count.
        quantity: u16,
    },
    /// Probe interval must not be zero.
    #[error("priority probe interval must be nonzero")]
    ZeroInterval,
    /// Probe operation timeout must not be zero.
    #[error("priority probe timeout must be nonzero")]
    ZeroTimeout,
}

/// Errors that prevent a raw pool lease from becoming a reusable client session.
#[cfg(feature = "client")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ReusableClientHandoffError {
    /// Direct access to either raw transport half permanently disqualified the lease.
    #[error("raw transport access disqualifies reusable client handoff")]
    RawTransportAccessed,

    /// The lease transport was already retired and is no longer available.
    #[error("pooled connection is no longer available for reusable client handoff")]
    LeaseUnavailable,
}
