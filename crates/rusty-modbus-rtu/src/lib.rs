//! Modbus RTU transport layer.
//!
//! Provides serial and RTU-over-TCP transports that implement the
//! `TransportSink`/`TransportStream` traits from `modbus-tcp`, allowing
//! RTU devices to be used interchangeably with TCP endpoints.

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all, clippy::pedantic)]

pub mod assembler;
pub mod config;
pub mod error;
pub mod rtu_tcp;
#[cfg(feature = "serial")]
pub mod serial;
pub mod unit_id;

pub use assembler::{
    AssemblerDeadline, AssemblerDiagnostics, AssemblerDiscardReason, AssemblerError,
    AssemblerOutcome, AssemblerRecovery, AssemblerState, OwnedRtuAdu, RtuFrameAssembler,
    RtuTimestamp, RtuTiming, RtuTimingError,
};
pub use config::{ResolvedRtuConfig, RtuConfig, RtuSerialFormat, RtuTimingMode, StrictRtuConfig};
pub use error::{RtuConfigError, RtuError};
pub use rtu_tcp::{RtuOverTcpTransport, RtuTcpRecvStream, RtuTcpSink};
#[cfg(feature = "serial")]
pub use serial::{SerialRecvStream, SerialSink, SerialTransport};
pub use unit_id::{RtuUnitIdError, RtuUnitIdRole};
