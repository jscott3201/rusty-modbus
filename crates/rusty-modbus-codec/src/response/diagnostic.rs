//! Response types for diagnostic function codes (FC 07, 08, 0B, 0C, 11).

use crate::error::{DecodeError, EncodeError};
use crate::request::Encode;
use rusty_modbus_types::{DiagnosticSubFunction, FunctionCode};

/// Response to a Read Exception Status request (FC 0x07).
#[derive(Debug)]
pub struct ReadExceptionStatusResponse {
    /// Eight exception status bits packed into one byte.
    pub status: u8,
}

impl ReadExceptionStatusResponse {
    /// Decode from the data bytes following the function code.
    ///
    /// # Errors
    ///
    /// Returns `DecodeError::Truncated` if `data` is too short.
    /// Returns `DecodeError::LengthMismatch` if `data` has extra bytes.
    pub fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        DecodeError::check_exact_len(data, 1)?;
        Ok(Self { status: data[0] })
    }
}

impl Encode for ReadExceptionStatusResponse {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        EncodeError::check_pdu_len(len)?;
        buf[0] = FunctionCode::ReadExceptionStatus.code();
        buf[1] = self.status;
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        1 + 1
    }
}

/// Response to a Diagnostics request (FC 0x08).
///
/// Echoes the sub-function code and associated data.
#[derive(Debug)]
pub struct DiagnosticsResponse<'buf> {
    /// The diagnostic sub-function code.
    pub sub_function: DiagnosticSubFunction,
    /// Diagnostic data bytes.
    pub data: &'buf [u8],
}

impl<'buf> DiagnosticsResponse<'buf> {
    /// Decode from the data bytes following the function code.
    ///
    /// # Errors
    ///
    /// Returns `DecodeError::Truncated` if `data` is too short.
    /// Returns `DecodeError::LengthMismatch` if `data` has extra bytes.
    pub fn decode(data: &'buf [u8]) -> Result<Self, DecodeError> {
        if data.len() < 2 {
            return Err(DecodeError::Truncated {
                expected: 2,
                actual: data.len(),
            });
        }
        let raw_sub = u16::from_be_bytes([data[0], data[1]]);
        let sub_function = DiagnosticSubFunction::from_raw(raw_sub)
            .ok_or(DecodeError::UnknownDiagnosticSubFunction(raw_sub))?;
        Ok(Self {
            sub_function,
            data: &data[2..],
        })
    }
}

impl Encode for DiagnosticsResponse<'_> {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        EncodeError::check_pdu_len(len)?;
        buf[0] = FunctionCode::Diagnostics.code();
        let sf = self.sub_function.code().to_be_bytes();
        buf[1] = sf[0];
        buf[2] = sf[1];
        buf[3..3 + self.data.len()].copy_from_slice(self.data);
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        1 + 2 + self.data.len()
    }
}

/// Response to a Get Comm Event Counter request (FC 0x0B).
#[derive(Debug)]
pub struct GetCommEventCounterResponse {
    /// Status word (0x0000 = ready, 0xFFFF = busy).
    pub status: u16,
    /// Event counter value.
    pub event_count: u16,
}

impl GetCommEventCounterResponse {
    /// Decode from the data bytes following the function code.
    ///
    /// # Errors
    ///
    /// Returns `DecodeError::Truncated` if `data` is too short.
    pub fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        DecodeError::check_exact_len(data, 4)?;
        let status = u16::from_be_bytes([data[0], data[1]]);
        let event_count = u16::from_be_bytes([data[2], data[3]]);
        Ok(Self {
            status,
            event_count,
        })
    }
}

impl Encode for GetCommEventCounterResponse {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        EncodeError::check_pdu_len(len)?;
        buf[0] = FunctionCode::GetCommEventCounter.code();
        let st = self.status.to_be_bytes();
        buf[1] = st[0];
        buf[2] = st[1];
        let ec = self.event_count.to_be_bytes();
        buf[3] = ec[0];
        buf[4] = ec[1];
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        1 + 4
    }
}

/// Response to a Get Comm Event Log request (FC 0x0C).
#[derive(Debug)]
pub struct GetCommEventLogResponse<'buf> {
    /// Number of bytes that follow.
    pub byte_count: u8,
    /// Status word (0x0000 = ready, 0xFFFF = busy).
    pub status: u16,
    /// Event counter value.
    pub event_count: u16,
    /// Message counter value.
    pub message_count: u16,
    /// Event log bytes.
    pub events: &'buf [u8],
}

impl<'buf> GetCommEventLogResponse<'buf> {
    /// Decode from the data bytes following the function code.
    ///
    /// # Errors
    ///
    /// Returns `DecodeError::Truncated` if `data` is too short.
    /// Returns `DecodeError::ByteCountMismatch` if the declared byte count
    /// does not match the remaining data length.
    pub fn decode(data: &'buf [u8]) -> Result<Self, DecodeError> {
        if data.len() < 7 {
            return Err(DecodeError::Truncated {
                expected: 7,
                actual: data.len(),
            });
        }
        let byte_count = data[0];
        let status = u16::from_be_bytes([data[1], data[2]]);
        let event_count = u16::from_be_bytes([data[3], data[4]]);
        let message_count = u16::from_be_bytes([data[5], data[6]]);
        let events = &data[7..];
        // byte_count covers status(2) + event_count(2) + message_count(2) + events.
        // Must be at least 6 to account for the three fixed 16-bit fields.
        if byte_count < 6 {
            return Err(DecodeError::ByteCountMismatch {
                declared: usize::from(byte_count),
                actual: 6 + events.len(),
            });
        }
        let declared_events_len = usize::from(byte_count) - 6;
        if events.len() != declared_events_len {
            return Err(DecodeError::ByteCountMismatch {
                declared: usize::from(byte_count),
                actual: 6 + events.len(),
            });
        }
        Ok(Self {
            byte_count,
            status,
            event_count,
            message_count,
            events,
        })
    }
}

impl Encode for GetCommEventLogResponse<'_> {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        EncodeError::check_byte_count(usize::from(self.byte_count), 6 + self.events.len())?;
        EncodeError::check_pdu_len(len)?;
        buf[0] = FunctionCode::GetCommEventLog.code();
        buf[1] = self.byte_count;
        let st = self.status.to_be_bytes();
        buf[2] = st[0];
        buf[3] = st[1];
        let ec = self.event_count.to_be_bytes();
        buf[4] = ec[0];
        buf[5] = ec[1];
        let mc = self.message_count.to_be_bytes();
        buf[6] = mc[0];
        buf[7] = mc[1];
        buf[8..8 + self.events.len()].copy_from_slice(self.events);
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        // FC + byte_count + status(2) + event_count(2) + message_count(2) + events
        1 + 1 + 6 + self.events.len()
    }
}

/// Response to a Report Server ID request (FC 0x11).
#[derive(Debug)]
pub struct ReportServerIdResponse<'buf> {
    /// Number of data bytes that follow.
    pub byte_count: u8,
    /// Device-specific identification data.
    pub data: &'buf [u8],
}

impl<'buf> ReportServerIdResponse<'buf> {
    /// Decode from the data bytes following the function code.
    ///
    /// # Errors
    ///
    /// Returns `DecodeError::Truncated` if `data` is too short.
    /// Returns `DecodeError::ByteCountMismatch` if the declared byte count
    /// does not match the remaining data length.
    pub fn decode(data: &'buf [u8]) -> Result<Self, DecodeError> {
        if data.is_empty() {
            return Err(DecodeError::Truncated {
                expected: 1,
                actual: 0,
            });
        }
        let byte_count = data[0];
        let payload = &data[1..];
        if payload.len() != usize::from(byte_count) {
            return Err(DecodeError::ByteCountMismatch {
                declared: usize::from(byte_count),
                actual: payload.len(),
            });
        }
        Ok(Self {
            byte_count,
            data: payload,
        })
    }
}

impl Encode for ReportServerIdResponse<'_> {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        EncodeError::check_byte_count(usize::from(self.byte_count), self.data.len())?;
        EncodeError::check_pdu_len(len)?;
        buf[0] = FunctionCode::ReportServerId.code();
        buf[1] = self.byte_count;
        buf[2..2 + usize::from(self.byte_count)].copy_from_slice(self.data);
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        1 + 1 + usize::from(self.byte_count)
    }
}
