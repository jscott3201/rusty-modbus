//! High-level async Modbus client with transaction pipelining.
//!
//! Supports concurrent in-flight requests matched by Transaction ID,
//! bounded retries for replay-safe requests and explicit Server Device Busy
//! responses, and typed methods for every supported client function code.

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all, clippy::pedantic)]

pub mod client;
pub mod config;
pub mod error;
mod methods;
pub(crate) mod reader;
pub(crate) mod transaction;

pub use client::ModbusClient;
pub use config::{ClientConfig, RetryConfig};
pub use error::ClientError;
