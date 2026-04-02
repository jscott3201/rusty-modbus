//! Owned (`'static`, `Bytes`-backed) Modbus response types.
//!
//! These mirror the borrowed response types in `rusty_modbus_codec::response` but hold
//! variable-length payloads as `bytes::Bytes` instead of `&'buf [u8]`, making them
//! suitable for async contexts where the response must outlive the receive buffer.

// from_pdu takes Bytes by value intentionally — callers transfer ownership of the
// buffer into the owned struct, even though Bytes::slice() only needs &self.
#![allow(clippy::needless_pass_by_value)]

use bytes::Bytes;
use rusty_modbus_codec::error::DecodeError;
use rusty_modbus_codec::response::{
    ExceptionResponse, GetCommEventCounterResponse, MaskWriteRegisterResponse,
    ReadExceptionStatusResponse, WriteMultipleCoilsResponse, WriteMultipleRegistersResponse,
    WriteSingleCoilResponse, WriteSingleRegisterResponse,
};
use rusty_modbus_types::{DiagnosticSubFunction, FunctionCode, MeiType};

// ---------------------------------------------------------------------------
// Owned bit-read responses
// ---------------------------------------------------------------------------

/// Owned variant of `ReadCoilsResponse` (FC 0x01).
#[derive(Debug, Clone)]
pub struct OwnedReadCoilsResponse {
    /// Number of data bytes that follow.
    pub byte_count: u8,
    /// Bit-packed coil status, LSB-first within each byte.
    pub coil_status: Bytes,
}

impl OwnedReadCoilsResponse {
    /// Decode from a full PDU (function-code byte + data).
    ///
    /// # Errors
    ///
    /// Returns `DecodeError` if the PDU is malformed.
    pub fn from_pdu(pdu: Bytes) -> Result<Self, DecodeError> {
        let data = &pdu[1..];
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
        let coil_status = pdu.slice(2..2 + usize::from(byte_count));
        Ok(Self {
            byte_count,
            coil_status,
        })
    }

    /// Returns the state of the coil at the given zero-based index.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range for the coil status data.
    #[must_use]
    pub fn coil(&self, index: usize) -> bool {
        let byte_idx = index / 8;
        let bit_idx = index % 8;
        (self.coil_status[byte_idx] >> bit_idx) & 1 == 1
    }
}

/// Owned variant of `ReadDiscreteInputsResponse` (FC 0x02).
#[derive(Debug, Clone)]
pub struct OwnedReadDiscreteInputsResponse {
    /// Number of data bytes that follow.
    pub byte_count: u8,
    /// Bit-packed input status, LSB-first within each byte.
    pub input_status: Bytes,
}

impl OwnedReadDiscreteInputsResponse {
    /// Decode from a full PDU (function-code byte + data).
    ///
    /// # Errors
    ///
    /// Returns `DecodeError` if the PDU is malformed.
    pub fn from_pdu(pdu: Bytes) -> Result<Self, DecodeError> {
        let data = &pdu[1..];
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
        let input_status = pdu.slice(2..2 + usize::from(byte_count));
        Ok(Self {
            byte_count,
            input_status,
        })
    }

    /// Returns the state of the discrete input at the given zero-based index.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range for the input status data.
    #[must_use]
    pub fn coil(&self, index: usize) -> bool {
        let byte_idx = index / 8;
        let bit_idx = index % 8;
        (self.input_status[byte_idx] >> bit_idx) & 1 == 1
    }
}

// ---------------------------------------------------------------------------
// Owned register-read responses
// ---------------------------------------------------------------------------

/// Owned variant of `ReadHoldingRegistersResponse` (FC 0x03).
#[derive(Debug, Clone)]
pub struct OwnedReadHoldingRegistersResponse {
    /// Number of data bytes that follow (should be 2 * register count).
    pub byte_count: u8,
    /// Raw register data in big-endian byte order.
    pub register_data: Bytes,
}

impl OwnedReadHoldingRegistersResponse {
    /// Decode from a full PDU (function-code byte + data).
    ///
    /// # Errors
    ///
    /// Returns `DecodeError` if the PDU is malformed.
    pub fn from_pdu(pdu: Bytes) -> Result<Self, DecodeError> {
        let data = &pdu[1..];
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
        let register_data = pdu.slice(2..2 + usize::from(byte_count));
        Ok(Self {
            byte_count,
            register_data,
        })
    }

    /// Returns the number of registers in this response.
    #[must_use]
    pub fn count(&self) -> usize {
        self.register_data.len() / 2
    }

    /// Returns the register value at the given zero-based index.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    #[must_use]
    pub fn register(&self, index: usize) -> u16 {
        let off = index * 2;
        u16::from_be_bytes([self.register_data[off], self.register_data[off + 1]])
    }

    /// Returns an iterator over all register values.
    pub fn registers(&self) -> impl Iterator<Item = u16> + '_ {
        self.register_data
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
    }

    /// Returns the raw register data bytes.
    #[must_use]
    pub fn raw(&self) -> &[u8] {
        &self.register_data
    }
}

/// Owned variant of `ReadInputRegistersResponse` (FC 0x04).
#[derive(Debug, Clone)]
pub struct OwnedReadInputRegistersResponse {
    /// Number of data bytes that follow (should be 2 * register count).
    pub byte_count: u8,
    /// Raw register data in big-endian byte order.
    pub register_data: Bytes,
}

impl OwnedReadInputRegistersResponse {
    /// Decode from a full PDU (function-code byte + data).
    ///
    /// # Errors
    ///
    /// Returns `DecodeError` if the PDU is malformed.
    pub fn from_pdu(pdu: Bytes) -> Result<Self, DecodeError> {
        let data = &pdu[1..];
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
        let register_data = pdu.slice(2..2 + usize::from(byte_count));
        Ok(Self {
            byte_count,
            register_data,
        })
    }

    /// Returns the number of registers in this response.
    #[must_use]
    pub fn count(&self) -> usize {
        self.register_data.len() / 2
    }

    /// Returns the register value at the given zero-based index.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    #[must_use]
    pub fn register(&self, index: usize) -> u16 {
        let off = index * 2;
        u16::from_be_bytes([self.register_data[off], self.register_data[off + 1]])
    }

    /// Returns an iterator over all register values.
    pub fn registers(&self) -> impl Iterator<Item = u16> + '_ {
        self.register_data
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
    }

    /// Returns the raw register data bytes.
    #[must_use]
    pub fn raw(&self) -> &[u8] {
        &self.register_data
    }
}

// ---------------------------------------------------------------------------
// Owned register read-write response
// ---------------------------------------------------------------------------

/// Owned variant of `ReadWriteMultipleRegistersResponse` (FC 0x17).
#[derive(Debug, Clone)]
pub struct OwnedReadWriteMultipleRegistersResponse {
    /// Number of data bytes that follow (should be 2 * register count).
    pub byte_count: u8,
    /// Raw register data in big-endian byte order.
    pub register_data: Bytes,
}

impl OwnedReadWriteMultipleRegistersResponse {
    /// Decode from a full PDU (function-code byte + data).
    ///
    /// # Errors
    ///
    /// Returns `DecodeError` if the PDU is malformed.
    pub fn from_pdu(pdu: Bytes) -> Result<Self, DecodeError> {
        let data = &pdu[1..];
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
        let register_data = pdu.slice(2..2 + usize::from(byte_count));
        Ok(Self {
            byte_count,
            register_data,
        })
    }

    /// Returns the number of registers in this response.
    #[must_use]
    pub fn count(&self) -> usize {
        self.register_data.len() / 2
    }

    /// Returns the register value at the given zero-based index.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    #[must_use]
    pub fn register(&self, index: usize) -> u16 {
        let off = index * 2;
        u16::from_be_bytes([self.register_data[off], self.register_data[off + 1]])
    }

    /// Returns an iterator over all register values.
    pub fn registers(&self) -> impl Iterator<Item = u16> + '_ {
        self.register_data
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
    }

    /// Returns the raw register data bytes.
    #[must_use]
    pub fn raw(&self) -> &[u8] {
        &self.register_data
    }
}

// ---------------------------------------------------------------------------
// Owned FIFO response
// ---------------------------------------------------------------------------

/// Owned variant of `ReadFifoQueueResponse` (FC 0x18).
#[derive(Debug, Clone)]
pub struct OwnedReadFifoQueueResponse {
    /// Total number of bytes following this field.
    pub byte_count: u16,
    /// Number of FIFO register values (0..=31).
    pub fifo_count: u16,
    /// Raw FIFO register data in big-endian byte order.
    pub fifo_values: Bytes,
}

impl OwnedReadFifoQueueResponse {
    /// Decode from a full PDU (function-code byte + data).
    ///
    /// # Errors
    ///
    /// Returns `DecodeError` if the PDU is malformed.
    pub fn from_pdu(pdu: Bytes) -> Result<Self, DecodeError> {
        let data = &pdu[1..];
        if data.len() < 4 {
            return Err(DecodeError::Truncated {
                expected: 4,
                actual: data.len(),
            });
        }
        let byte_count = u16::from_be_bytes([data[0], data[1]]);
        let fifo_count = u16::from_be_bytes([data[2], data[3]]);
        let fifo_values_slice = &data[4..];
        // byte_count includes the 2 bytes for fifo_count + the fifo data
        let expected_remaining = usize::from(byte_count).saturating_sub(2);
        if fifo_values_slice.len() != expected_remaining {
            return Err(DecodeError::ByteCountMismatch {
                declared: expected_remaining,
                actual: fifo_values_slice.len(),
            });
        }
        let fifo_values = pdu.slice(5..5 + expected_remaining);
        Ok(Self {
            byte_count,
            fifo_count,
            fifo_values,
        })
    }
}

// ---------------------------------------------------------------------------
// Owned file record responses
// ---------------------------------------------------------------------------

/// Owned variant of `ReadFileRecordResponse` (FC 0x14).
#[derive(Debug, Clone)]
pub struct OwnedReadFileRecordResponse {
    /// Total number of data bytes that follow.
    pub byte_count: u8,
    /// Raw sub-request response data.
    pub data: Bytes,
}

impl OwnedReadFileRecordResponse {
    /// Decode from a full PDU (function-code byte + data).
    ///
    /// # Errors
    ///
    /// Returns `DecodeError` if the PDU is malformed.
    pub fn from_pdu(pdu: Bytes) -> Result<Self, DecodeError> {
        let data = &pdu[1..];
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
        let owned_data = pdu.slice(2..2 + usize::from(byte_count));
        Ok(Self {
            byte_count,
            data: owned_data,
        })
    }
}

/// Owned variant of `WriteFileRecordResponse` (FC 0x15).
#[derive(Debug, Clone)]
pub struct OwnedWriteFileRecordResponse {
    /// Total number of data bytes that follow.
    pub byte_count: u8,
    /// Raw sub-request response data.
    pub data: Bytes,
}

impl OwnedWriteFileRecordResponse {
    /// Decode from a full PDU (function-code byte + data).
    ///
    /// # Errors
    ///
    /// Returns `DecodeError` if the PDU is malformed.
    pub fn from_pdu(pdu: Bytes) -> Result<Self, DecodeError> {
        let data = &pdu[1..];
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
        let owned_data = pdu.slice(2..2 + usize::from(byte_count));
        Ok(Self {
            byte_count,
            data: owned_data,
        })
    }
}

// ---------------------------------------------------------------------------
// Owned diagnostic responses
// ---------------------------------------------------------------------------

/// Owned variant of `DiagnosticsResponse` (FC 0x08).
#[derive(Debug, Clone)]
pub struct OwnedDiagnosticsResponse {
    /// The diagnostic sub-function code.
    pub sub_function: DiagnosticSubFunction,
    /// Diagnostic data bytes.
    pub data: Bytes,
}

impl OwnedDiagnosticsResponse {
    /// Decode from a full PDU (function-code byte + data).
    ///
    /// # Errors
    ///
    /// Returns `DecodeError` if the PDU is malformed.
    pub fn from_pdu(pdu: Bytes) -> Result<Self, DecodeError> {
        let data = &pdu[1..];
        if data.len() < 2 {
            return Err(DecodeError::Truncated {
                expected: 2,
                actual: data.len(),
            });
        }
        let raw_sub = u16::from_be_bytes([data[0], data[1]]);
        let sub_function = DiagnosticSubFunction::from_raw(raw_sub)
            .ok_or(DecodeError::UnknownFunctionCode(data[0]))?;
        let owned_data = pdu.slice(3..);
        Ok(Self {
            sub_function,
            data: owned_data,
        })
    }
}

/// Owned variant of `GetCommEventLogResponse` (FC 0x0C).
#[derive(Debug, Clone)]
pub struct OwnedGetCommEventLogResponse {
    /// Number of bytes that follow.
    pub byte_count: u8,
    /// Status word (0x0000 = ready, 0xFFFF = busy).
    pub status: u16,
    /// Event counter value.
    pub event_count: u16,
    /// Message counter value.
    pub message_count: u16,
    /// Event log bytes.
    pub events: Bytes,
}

impl OwnedGetCommEventLogResponse {
    /// Decode from a full PDU (function-code byte + data).
    ///
    /// # Errors
    ///
    /// Returns `DecodeError` if the PDU is malformed.
    pub fn from_pdu(pdu: Bytes) -> Result<Self, DecodeError> {
        let data = &pdu[1..];
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
        let events_slice = &data[7..];
        // byte_count covers status(2) + event_count(2) + message_count(2) + events
        let declared_events_len = usize::from(byte_count).saturating_sub(6);
        if events_slice.len() != declared_events_len {
            return Err(DecodeError::ByteCountMismatch {
                declared: usize::from(byte_count),
                actual: 6 + events_slice.len(),
            });
        }
        let events = pdu.slice(8..8 + declared_events_len);
        Ok(Self {
            byte_count,
            status,
            event_count,
            message_count,
            events,
        })
    }
}

/// Owned variant of `ReportServerIdResponse` (FC 0x11).
#[derive(Debug, Clone)]
pub struct OwnedReportServerIdResponse {
    /// Number of data bytes that follow.
    pub byte_count: u8,
    /// Device-specific identification data.
    pub data: Bytes,
}

impl OwnedReportServerIdResponse {
    /// Decode from a full PDU (function-code byte + data).
    ///
    /// # Errors
    ///
    /// Returns `DecodeError` if the PDU is malformed.
    pub fn from_pdu(pdu: Bytes) -> Result<Self, DecodeError> {
        let data = &pdu[1..];
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
        let owned_data = pdu.slice(2..2 + usize::from(byte_count));
        Ok(Self {
            byte_count,
            data: owned_data,
        })
    }
}

// ---------------------------------------------------------------------------
// Owned MEI response
// ---------------------------------------------------------------------------

/// Owned variant of `EncapsulatedInterfaceResponse` (FC 0x2B).
#[derive(Debug, Clone)]
pub struct OwnedEncapsulatedInterfaceResponse {
    /// The MEI type code.
    pub mei_type: MeiType,
    /// MEI-specific data bytes.
    pub data: Bytes,
}

impl OwnedEncapsulatedInterfaceResponse {
    /// Decode from a full PDU (function-code byte + data).
    ///
    /// # Errors
    ///
    /// Returns `DecodeError` if the PDU is malformed.
    pub fn from_pdu(pdu: Bytes) -> Result<Self, DecodeError> {
        let data = &pdu[1..];
        if data.is_empty() {
            return Err(DecodeError::Truncated {
                expected: 1,
                actual: 0,
            });
        }
        let mei_type = MeiType::from_raw(data[0]).ok_or(DecodeError::UnknownMeiType(data[0]))?;
        let owned_data = pdu.slice(2..);
        Ok(Self {
            mei_type,
            data: owned_data,
        })
    }
}

// ---------------------------------------------------------------------------
// Collected device identification
// ---------------------------------------------------------------------------

/// Collected device identification from FC 0x2B / MEI 0x0E.
///
/// Contains the three mandatory basic objects. Fields are `None` if the
/// device did not include them.
#[derive(Debug, Clone, Default)]
pub struct OwnedDeviceIdentification {
    /// Vendor Name (object 0x00).
    pub vendor_name: Option<String>,
    /// Product Code (object 0x01).
    pub product_code: Option<String>,
    /// Major/Minor Revision (object 0x02).
    pub major_minor_revision: Option<String>,
}

// ---------------------------------------------------------------------------
// OwnedResponsePdu dispatch enum
// ---------------------------------------------------------------------------

/// Owned variant of a Modbus response PDU.
///
/// Variable-length responses use `Bytes`-backed owned types; fixed-size (Copy)
/// responses are used directly from `rusty_modbus_codec::response`.
#[derive(Debug)]
pub enum OwnedResponsePdu {
    /// FC 0x01 -- Read Coils.
    ReadCoils(OwnedReadCoilsResponse),
    /// FC 0x02 -- Read Discrete Inputs.
    ReadDiscreteInputs(OwnedReadDiscreteInputsResponse),
    /// FC 0x03 -- Read Holding Registers.
    ReadHoldingRegisters(OwnedReadHoldingRegistersResponse),
    /// FC 0x04 -- Read Input Registers.
    ReadInputRegisters(OwnedReadInputRegistersResponse),
    /// FC 0x05 -- Write Single Coil.
    WriteSingleCoil(WriteSingleCoilResponse),
    /// FC 0x06 -- Write Single Register.
    WriteSingleRegister(WriteSingleRegisterResponse),
    /// FC 0x07 -- Read Exception Status.
    ReadExceptionStatus(ReadExceptionStatusResponse),
    /// FC 0x08 -- Diagnostics.
    Diagnostics(OwnedDiagnosticsResponse),
    /// FC 0x0B -- Get Comm Event Counter.
    GetCommEventCounter(GetCommEventCounterResponse),
    /// FC 0x0C -- Get Comm Event Log.
    GetCommEventLog(OwnedGetCommEventLogResponse),
    /// FC 0x0F -- Write Multiple Coils.
    WriteMultipleCoils(WriteMultipleCoilsResponse),
    /// FC 0x10 -- Write Multiple Registers.
    WriteMultipleRegisters(WriteMultipleRegistersResponse),
    /// FC 0x11 -- Report Server ID.
    ReportServerId(OwnedReportServerIdResponse),
    /// FC 0x14 -- Read File Record.
    ReadFileRecord(OwnedReadFileRecordResponse),
    /// FC 0x15 -- Write File Record.
    WriteFileRecord(OwnedWriteFileRecordResponse),
    /// FC 0x16 -- Mask Write Register.
    MaskWriteRegister(MaskWriteRegisterResponse),
    /// FC 0x17 -- Read/Write Multiple Registers.
    ReadWriteMultipleRegisters(OwnedReadWriteMultipleRegistersResponse),
    /// FC 0x18 -- Read FIFO Queue.
    ReadFifoQueue(OwnedReadFifoQueueResponse),
    /// FC 0x2B -- Encapsulated Interface Transport.
    EncapsulatedInterface(OwnedEncapsulatedInterfaceResponse),
    /// Non-standard / vendor-specific response.
    Custom(u8, Bytes),
    /// Exception response (FC | 0x80).
    Exception(ExceptionResponse),
}

impl OwnedResponsePdu {
    /// Decode from a full PDU (`Bytes` starting with the function-code byte).
    ///
    /// Dispatches to the appropriate owned type based on the function code.
    ///
    /// # Errors
    ///
    /// Returns `DecodeError` if the function code is unknown or the payload is
    /// malformed.
    pub fn from_pdu(pdu: Bytes) -> Result<Self, DecodeError> {
        if pdu.is_empty() {
            return Err(DecodeError::Truncated {
                expected: 1,
                actual: 0,
            });
        }

        let fc_byte = pdu[0];

        // Check for exception response (high bit set).
        if FunctionCode::is_exception_response(fc_byte) {
            let resp = ExceptionResponse::decode(fc_byte, &pdu[1..])?;
            return Ok(Self::Exception(resp));
        }

        // Exception-flagged bytes handled above. from_raw returns Some for all
        // non-exception bytes (known → named, unknown → Custom).
        let fc = FunctionCode::from_raw(fc_byte).unwrap_or(FunctionCode::Custom(fc_byte));

        let data = &pdu[1..];

        match fc {
            FunctionCode::ReadCoils => OwnedReadCoilsResponse::from_pdu(pdu).map(Self::ReadCoils),
            FunctionCode::ReadDiscreteInputs => {
                OwnedReadDiscreteInputsResponse::from_pdu(pdu).map(Self::ReadDiscreteInputs)
            }
            FunctionCode::ReadHoldingRegisters => {
                OwnedReadHoldingRegistersResponse::from_pdu(pdu).map(Self::ReadHoldingRegisters)
            }
            FunctionCode::ReadInputRegisters => {
                OwnedReadInputRegistersResponse::from_pdu(pdu).map(Self::ReadInputRegisters)
            }
            FunctionCode::WriteSingleCoil => {
                WriteSingleCoilResponse::decode(data).map(Self::WriteSingleCoil)
            }
            FunctionCode::WriteSingleRegister => {
                WriteSingleRegisterResponse::decode(data).map(Self::WriteSingleRegister)
            }
            FunctionCode::ReadExceptionStatus => {
                ReadExceptionStatusResponse::decode(data).map(Self::ReadExceptionStatus)
            }
            FunctionCode::Diagnostics => {
                OwnedDiagnosticsResponse::from_pdu(pdu).map(Self::Diagnostics)
            }
            FunctionCode::GetCommEventCounter => {
                GetCommEventCounterResponse::decode(data).map(Self::GetCommEventCounter)
            }
            FunctionCode::GetCommEventLog => {
                OwnedGetCommEventLogResponse::from_pdu(pdu).map(Self::GetCommEventLog)
            }
            FunctionCode::WriteMultipleCoils => {
                WriteMultipleCoilsResponse::decode(data).map(Self::WriteMultipleCoils)
            }
            FunctionCode::WriteMultipleRegisters => {
                WriteMultipleRegistersResponse::decode(data).map(Self::WriteMultipleRegisters)
            }
            FunctionCode::ReportServerId => {
                OwnedReportServerIdResponse::from_pdu(pdu).map(Self::ReportServerId)
            }
            FunctionCode::ReadFileRecord => {
                OwnedReadFileRecordResponse::from_pdu(pdu).map(Self::ReadFileRecord)
            }
            FunctionCode::WriteFileRecord => {
                OwnedWriteFileRecordResponse::from_pdu(pdu).map(Self::WriteFileRecord)
            }
            FunctionCode::MaskWriteRegister => {
                MaskWriteRegisterResponse::decode(data).map(Self::MaskWriteRegister)
            }
            FunctionCode::ReadWriteMultipleRegisters => {
                OwnedReadWriteMultipleRegistersResponse::from_pdu(pdu)
                    .map(Self::ReadWriteMultipleRegisters)
            }
            FunctionCode::ReadFifoQueue => {
                OwnedReadFifoQueueResponse::from_pdu(pdu).map(Self::ReadFifoQueue)
            }
            FunctionCode::EncapsulatedInterfaceTransport => {
                OwnedEncapsulatedInterfaceResponse::from_pdu(pdu).map(Self::EncapsulatedInterface)
            }
            FunctionCode::Custom(fc) => Ok(Self::Custom(fc, pdu.slice(1..))),
        }
    }
}
