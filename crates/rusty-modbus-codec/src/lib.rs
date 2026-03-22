//! Sans-IO Modbus PDU codec — zero-copy decode and encode for all public function codes.
//!
//! `no_std` by default, no allocator required. Operates purely on `&[u8]` (decode)
//! and `&mut [u8]` (encode), composable with any transport.

#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all, clippy::pedantic)]

#[cfg(feature = "std")]
extern crate std;

#[cfg(test)]
extern crate alloc;

pub mod decode;
pub mod error;
pub mod pdu;
pub mod request;
pub mod response;
pub mod validate;

pub use decode::{decode_pdu_ref, decode_request, decode_response};
pub use error::{DecodeError, EncodeError};
pub use pdu::{PduRef, RequestPdu, ResponsePdu};
pub use request::Encode;
