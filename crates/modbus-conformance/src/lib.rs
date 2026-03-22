//! Modbus protocol conformance tests.
//!
//! Tests are driven by the Modbus Application Protocol Specification V1.1b3
//! and verify `modbus-types` and `modbus-codec` against authoritative wire
//! format examples and validation state diagrams.

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all, clippy::pedantic)]
