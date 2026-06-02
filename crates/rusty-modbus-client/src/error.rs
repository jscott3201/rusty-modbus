//! Client error types.

use rusty_modbus_codec::response::ExceptionResponse;
use rusty_modbus_codec::{DecodeError, EncodeError};
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

    /// Request could not be encoded because caller-supplied arguments violate
    /// Modbus wire limits.
    #[error("request encode error: {0}")]
    Encode(#[from] EncodeError),

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

    /// The server's response function code did not echo the request's
    /// (Modbus V1.1b3 §4.4: a normal response repeats the FC, an error sets
    /// `fc | 0x80`). Guards against a stale or misrouted frame being delivered
    /// for a transaction slot it does not belong to.
    #[error("unexpected response function code: expected {expected:#04x}, got {got:#04x}")]
    UnexpectedResponse {
        /// The function code that was requested.
        expected: u8,
        /// The function code the server actually returned.
        got: u8,
    },

    /// The server's write response did not echo a request field required by
    /// the Modbus function definition. Write responses for FC05, FC06, FC0F,
    /// FC10, and FC16 are confirmations of the requested address/value/masks;
    /// accepting a different echo would report success for the wrong mutation.
    #[error("unexpected response echo for {field}: expected {expected:#06x}, got {got:#06x}")]
    UnexpectedResponseEcho {
        /// Field that failed to echo, such as `address`, `value`, or `quantity`.
        field: &'static str,
        /// Requested field value.
        expected: u16,
        /// Field value returned by the server.
        got: u16,
    },

    /// The server returned fewer data bytes than the requested quantity needs.
    /// Rejecting this prevents a malicious or buggy peer from truncating a read
    /// (and, for bit reads, from triggering an out-of-bounds index).
    #[error("short response: need {expected} data bytes for the request, got {actual}")]
    ShortResponse {
        /// Data bytes required to satisfy the request.
        expected: usize,
        /// Data bytes the server actually returned.
        actual: usize,
    },
}
