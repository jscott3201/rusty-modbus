//! Transport split traits for concurrent pipelining.
//!
//! Transports expose independent read/write halves so the client can send
//! requests while concurrently reading responses. Uses native async fn in
//! traits (RPITIT, Rust 1.85+) — no `async_trait` crate.

use std::future::Future;

use modbus_frame::Frame;

use crate::error::TransportError;

/// Write half of a transport — sends framed PDUs.
pub trait TransportSink: Send {
    /// Send a complete frame to the remote peer.
    fn send(&mut self, frame: Frame) -> impl Future<Output = Result<(), TransportError>> + Send;
}

/// Read half of a transport — receives framed PDUs.
pub trait TransportStream: Send {
    /// Receive the next complete frame from the remote peer.
    ///
    /// Returns `Err(TransportError::Disconnected)` when the connection is closed.
    fn recv(&mut self) -> impl Future<Output = Result<Frame, TransportError>> + Send;
}

/// Factory for creating connected transport halves.
pub trait TransportConnect: Send {
    /// The write half type.
    type Sink: TransportSink;
    /// The read half type.
    type Stream: TransportStream;

    /// Establish a connection and return split (sink, stream) halves.
    fn connect(
        config: crate::config::TcpConfig,
        addr: std::net::SocketAddr,
    ) -> impl Future<Output = Result<(Self::Sink, Self::Stream), TransportError>> + Send;
}
