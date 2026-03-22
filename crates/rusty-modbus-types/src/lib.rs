//! Foundation types for the Modbus protocol family.
//!
//! `no_std` by default, no allocator required. Every type, constant, and
//! wire-format struct needed by the Modbus protocol stack lives here.

#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all, clippy::pedantic)]

#[cfg(feature = "std")]
extern crate std;

#[cfg(test)]
extern crate alloc;

pub mod coil;
pub mod constants;
pub mod diagnostic;
pub mod error;
pub mod exception;
pub mod function;
pub mod header;
pub mod mei;
pub mod model;

pub use coil::CoilValue;
pub use constants::*;
pub use diagnostic::DiagnosticSubFunction;
pub use error::TypeError;
pub use exception::ExceptionCode;
pub use function::FunctionCode;
pub use header::MbapHeader;
pub use mei::{DeviceIdCode, DeviceIdObject, MeiType};
pub use model::{Address, DataTable, Quantity, TransactionId, UnitId};
