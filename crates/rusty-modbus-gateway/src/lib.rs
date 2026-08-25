//! Modbus TCP ↔ RTU gateway.
//!
//! Bridges TCP clients to serial (RTU) devices by routing requests
//! based on Unit ID and translating between MBAP and RTU framing.

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all, clippy::pedantic)]
#![allow(clippy::missing_errors_doc)]

pub mod config;
pub mod error;
pub mod gateway;
pub mod routing;
pub mod translator;

pub use config::{GatewayConfig, RouteEntry};
pub use error::GatewayError;
pub use gateway::ModbusGateway;
pub use routing::RouteTable;
pub use rusty_modbus_rtu::RtuOverTcpFramingPolicy;
