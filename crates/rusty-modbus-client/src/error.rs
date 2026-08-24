//! Client error types.

use rusty_modbus_codec::response::ExceptionResponse;
use rusty_modbus_codec::{DecodeError, EncodeError};
use rusty_modbus_tcp::TransportError;
use rusty_modbus_types::TransactionId;

/// Errors that can occur during client operations.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// One request attempt reached its deadline.
    ///
    /// For a mutating request, this error does not prove that the server did
    /// not execute the write.
    #[error("request timed out")]
    Timeout,

    /// Server returned a Modbus exception.
    ///
    /// `Acknowledge` (`0x05`) means the server accepted the request and is still
    /// processing it. The client returns that code here without treating it as
    /// completed success or replaying the request.
    #[error("Modbus exception: {0:?}")]
    Exception(ExceptionResponse),

    /// Transport-level error (disconnect, I/O, framing).
    ///
    /// A send error does not prove that the transport wrote zero request bytes.
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

    /// A Modbus/TCP response used the request's transaction ID but carried a
    /// different Unit Identifier.
    #[error("unexpected response unit ID: expected {expected:#04x}, got {got:#04x}")]
    UnexpectedResponseUnitId {
        /// Unit Identifier carried by the request.
        expected: u8,
        /// Unit Identifier carried by the response.
        got: u8,
    },

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

    /// The server returned more data bytes than the initiating read requested.
    #[error(
        "unexpected response length for function {function_code:#04x}: expected {expected} data bytes, got {actual}"
    )]
    UnexpectedResponseLength {
        /// Function code of the initiating request.
        function_code: u8,
        /// Exact data length required by the request.
        expected: usize,
        /// Data bytes returned by the server.
        actual: usize,
    },

    /// A bit-read response set high bits outside the requested quantity.
    #[error(
        "unexpected response padding for function {function_code:#04x}: byte {actual:#04x} sets bits selected by {invalid_mask:#04x}"
    )]
    UnexpectedResponsePadding {
        /// Function code of the initiating request.
        function_code: u8,
        /// Mask selecting the unused high bits in the final byte.
        invalid_mask: u8,
        /// Final response byte containing invalid padding.
        actual: u8,
    },

    /// A Read Device Identification continuation did not advance to a higher
    /// object ID. Without this guard, a malformed peer can keep the client in
    /// an unbounded `more_follows` loop.
    #[error(
        "invalid device identification continuation: next object ID {next_object_id:#04x} did not advance past {previous_object_id:#04x}"
    )]
    InvalidDeviceIdentificationContinuation {
        /// Object ID used for the request that produced the response.
        previous_object_id: u8,
        /// Continuation object ID returned by the server.
        next_object_id: u8,
    },

    /// A Read Device Identification response chain exceeded the maximum number
    /// of Basic identification pages the client will follow.
    #[error("device identification pagination exceeded {limit} response pages")]
    DeviceIdentificationPaginationLimit {
        /// Maximum number of response pages followed for the request.
        limit: u8,
    },
}
