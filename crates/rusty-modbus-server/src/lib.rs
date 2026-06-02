//! Async Modbus server with pluggable data store.
//!
//! Handles incoming requests, validates per spec state machines, and
//! reads/writes from a configurable data store.

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all, clippy::pedantic)]
#![allow(clippy::needless_pass_by_value)] // API boundary types are intentionally consumed
#![allow(clippy::missing_errors_doc)] // Error conditions documented at trait level

pub mod config;
mod device_id;
pub mod error;
pub mod handler;
mod response_encode;
pub mod server;
pub mod store;

pub use config::{DeviceIdentification, ServerConfig};
pub use error::ServerError;
pub use server::ModbusServer;
pub use store::memory::{InMemoryStore, StoreConfig};
pub use store::{CommEventLog, DataStore};
