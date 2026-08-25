//! Modbus device simulator for testing and CI.
//!
//! Configures a Modbus/TCP server with validated static register maps and
//! pre-built device profiles.

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all, clippy::pedantic)]
#![allow(clippy::missing_errors_doc)]

pub mod config;
pub mod error;
pub mod scenario;
pub mod simulator;

pub use config::SimConfig;
pub use error::SimError;
pub use scenario::{generic_io, hvac_controller, power_meter, vfd_drive};
pub use simulator::ModbusSimulator;
