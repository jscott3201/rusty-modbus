//! Diagnostic requests: FC 07, 08, 0B, 0C, 11.
//!
//! FC 07 (Read Exception Status), FC 0B (Get Comm Event Counter),
//! FC 0C (Get Comm Event Log), and FC 11 (Report Server ID) have empty
//! request PDUs — no data follows the function code byte. They are
//! represented as unit variants in [`RequestPdu`](crate::pdu::RequestPdu).
//!
//! FC 08 (Diagnostics) carries a sub-function code and variable data.

use rusty_modbus_types::{DiagnosticSubFunction, FunctionCode};

use crate::error::{DecodeError, EncodeError};
use crate::request::Encode;

/// FC 0x08 — Diagnostics request.
///
/// The sub-function code selects the diagnostic action. Most sub-functions
/// carry a single 16-bit data word, but Return Query Data (0x0000) echoes
/// arbitrary data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticsRequest<'buf> {
    /// Diagnostic sub-function code.
    pub sub_function: DiagnosticSubFunction,
    /// Sub-function data (typically 2 bytes, but variable for Return Query Data).
    pub data: &'buf [u8],
}

impl<'buf> DiagnosticsRequest<'buf> {
    /// Decode from PDU data after the function code byte.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Truncated`] if `data` is shorter than 2 bytes.
    /// Returns [`DecodeError::UnknownFunctionCode`] if the sub-function code is not recognized
    /// (reused as a general "unknown sub-code" error via the raw value).
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
        let payload = &data[2..];
        Ok(Self {
            sub_function,
            data: payload,
        })
    }
}

impl Encode for DiagnosticsRequest<'_> {
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
        buf[1..3].copy_from_slice(&self.sub_function.code().to_be_bytes());
        buf[3..3 + self.data.len()].copy_from_slice(self.data);
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        // FC(1) + sub_function(2) + data
        3 + self.data.len()
    }
}
