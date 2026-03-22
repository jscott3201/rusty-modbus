//! Unified frame type for decoded Modbus frames.

use bytes::Bytes;
use modbus_types::MbapHeader;

/// A decoded Modbus frame from any transport.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Frame header (MBAP for TCP, unit ID for RTU).
    pub header: FrameHeader,
    /// PDU bytes (function code + data). Frozen `Bytes` handle — ref-counted, no copy.
    pub pdu: Bytes,
}

/// Frame header discriminant — identifies the transport source.
#[derive(Debug, Clone, Copy)]
pub enum FrameHeader {
    /// MBAP header from TCP transport.
    Mbap(MbapHeader),
    /// RTU header — just the unit ID (CRC already validated and stripped).
    Rtu {
        /// Unit/slave address byte.
        unit_id: u8,
    },
}

impl Frame {
    /// Extract the unit ID from any frame header.
    #[must_use]
    pub fn unit_id(&self) -> u8 {
        match &self.header {
            FrameHeader::Mbap(h) => h.unit_id,
            FrameHeader::Rtu { unit_id } => *unit_id,
        }
    }
}

/// RTU framing metadata (moved here from modbus-types per review finding #6).
#[derive(Debug, Clone, Copy)]
pub struct RtuFrameMeta {
    /// Unit/slave address.
    pub unit_id: u8,
    /// PDU byte length (excluding unit ID and CRC).
    pub pdu_length: usize,
    /// CRC-16 value from the frame.
    pub crc: u16,
}
