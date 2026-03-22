//! Modbus frame layer — bridges the `no_std` codec to tokio async.
//!
//! Implements `tokio_util::codec::Decoder`/`Encoder` for MBAP (TCP) and RTU framing.
//! Houses CRC-16/Modbus computation and owned (`'static`) response types backed by `Bytes`.

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all, clippy::pedantic)]

pub mod crc;
pub mod error;
pub mod frame;
pub mod mbap;
pub mod owned;
#[cfg(feature = "rtu")]
pub mod rtu;
#[cfg(feature = "rtu")]
pub mod rtu_tcp;

pub use crc::{crc16, verify_crc};
pub use error::FrameError;
pub use frame::{Frame, FrameHeader, RtuFrameMeta};
pub use mbap::MbapCodec;
pub use owned::{OwnedDeviceIdentification, OwnedResponsePdu};
