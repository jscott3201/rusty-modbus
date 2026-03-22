//! Modbus RTU transport layer.
//!
//! Provides serial and RTU-over-TCP transports that implement the
//! `TransportSink`/`TransportStream` traits from `modbus-tcp`, allowing
//! RTU devices to be used interchangeably with TCP endpoints.

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all, clippy::pedantic)]

pub mod config;
pub mod error;
pub mod rtu_tcp;
#[cfg(feature = "serial")]
pub mod serial;

pub use config::RtuConfig;
pub use error::RtuError;
pub use rtu_tcp::{RtuOverTcpTransport, RtuTcpRecvStream, RtuTcpSink};
#[cfg(feature = "serial")]
pub use serial::{SerialRecvStream, SerialSink, SerialTransport};
