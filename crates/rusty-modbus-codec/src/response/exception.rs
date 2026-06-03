//! Exception response type.

use crate::error::{DecodeError, EncodeError};
use crate::request::Encode;
use rusty_modbus_types::{ExceptionCode, FunctionCode};

/// Modbus exception response.
///
/// Sent by a server when a request cannot be processed. The function code
/// byte on the wire has bit 7 set (0x80 flag).
#[derive(Debug, Clone, Copy)]
pub struct ExceptionResponse {
    /// The original function code (without the 0x80 flag).
    pub function_code: FunctionCode,
    /// The exception code indicating the error condition.
    pub exception_code: ExceptionCode,
}

impl ExceptionResponse {
    /// Decode from the raw function code byte and remaining data.
    ///
    /// The `fc_byte` parameter is the raw byte from the wire WITH the 0x80
    /// exception flag still set. The flag is stripped internally using
    /// `FunctionCode::from_exception_raw`.
    ///
    /// # Errors
    ///
    /// Returns `DecodeError::Truncated` if `data` is too short.
    /// Returns `DecodeError::LengthMismatch` if `data` has extra bytes.
    /// Returns `DecodeError::UnknownFunctionCode` if the function code is not
    /// recognized.
    /// Returns `DecodeError::UnknownExceptionCode` if the exception code is
    /// not recognized.
    pub fn decode(fc_byte: u8, data: &[u8]) -> Result<Self, DecodeError> {
        DecodeError::check_exact_len(data, 1)?;
        let raw_function_code = fc_byte & 0x7F;
        if raw_function_code == 0 {
            return Err(DecodeError::UnknownFunctionCode(raw_function_code));
        }
        let function_code = FunctionCode::from_exception_raw(fc_byte);
        let exception_code = ExceptionCode::from_raw(data[0]);
        Ok(Self {
            function_code,
            exception_code,
        })
    }
}

impl Encode for ExceptionResponse {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        EncodeError::check_pdu_len(len)?;
        buf[0] = self.function_code.exception_code();
        buf[1] = self.exception_code.code();
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        2
    }
}
