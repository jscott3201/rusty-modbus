//! Async TCP transport for Modbus/TCP on port 502.
//!
//! Manages connection lifecycle, TCP socket options, and provides the
//! `TransportSink`/`TransportStream` trait implementations used by the
//! client and server crates.

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all, clippy::pedantic)]

pub mod config;
pub mod connect;
pub mod error;
pub mod listener;
pub mod transport;

pub use config::{AccessControl, AccessMode, TcpConfig, TcpServerConfig};
pub use connect::{TcpIdleObservation, TcpRecvStream, TcpSink, TcpTransport, inspect_idle_tcp};
pub use error::{TcpError, TransportError};
pub use listener::{
    ConnectionGuard, TcpServerListener, TcpServerMetrics, TcpServerMetricsSnapshot,
};
pub use transport::{TransportConnect, TransportSink, TransportStream};
