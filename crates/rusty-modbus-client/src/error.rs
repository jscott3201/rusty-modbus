//! Client error types.

use rusty_modbus_codec::DecodeError;
use rusty_modbus_codec::response::ExceptionResponse;
use rusty_modbus_tcp::TransportError;
use rusty_modbus_types::TransactionId;

/// Errors that can occur during client operations.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Request timed out waiting for response.
    #[error("request timed out")]
    Timeout,

    /// Server returned a Modbus exception.
    #[error("Modbus exception: {0:?}")]
    Exception(ExceptionResponse),

    /// Transport-level error (disconnect, I/O, framing).
    #[error("transport error: {0}")]
    Transport(#[from] TransportError),

    /// Codec encode/decode error.
    #[error("codec error: {0}")]
    Codec(#[from] DecodeError),

    /// Transaction ID collision — slot already occupied.
    #[error("transaction ID conflict: {0:?}")]
    TransactionConflict(TransactionId),

    /// Client is not connected.
    #[error("not connected")]
    NotConnected,

    /// Max retries exhausted.
    #[error("retries exhausted after {attempts} attempts: {last_error}")]
    RetriesExhausted {
        /// Number of attempts made.
        attempts: u32,
        /// The last error encountered.
        last_error: Box<ClientError>,
    },

    /// Read operation attempted on broadcast Unit ID (0x00).
    #[error("read operations are not allowed on broadcast unit ID (0x00)")]
    BroadcastReadNotAllowed,

    /// Client is shutting down — in-flight request was cancelled.
    #[error("client is shutting down")]
    ShuttingDown,
}
