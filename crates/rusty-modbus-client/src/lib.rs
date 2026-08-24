//! High-level async Modbus client with transaction pipelining.
//!
//! Supports concurrent in-flight requests matched by Transaction ID,
//! bounded retries for replay-safe requests and explicit Server Device Busy
//! responses, typed methods for every supported client function code, and
//! client-owned graceful shutdown or immediate abort.

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all, clippy::pedantic)]

pub mod client;
pub mod config;
pub mod error;
mod lifecycle;
mod methods;
pub(crate) mod reader;
pub(crate) mod transaction;

pub use client::ModbusClient;
pub use config::{ClientConfig, RetryConfig};
pub use error::ClientError;
