//! Top-level PDU decode dispatchers.

use rusty_modbus_types::{FunctionCode, MAX_PDU_SIZE};

use crate::error::DecodeError;
use crate::pdu::{PduRef, RequestPdu, ResponsePdu};
use crate::request::{
    DiagnosticsRequest, EncapsulatedInterfaceRequest, MaskWriteRegisterRequest, ReadCoilsRequest,
    ReadDiscreteInputsRequest, ReadFifoQueueRequest, ReadFileRecordRequest,
    ReadHoldingRegistersRequest, ReadInputRegistersRequest, ReadWriteMultipleRegistersRequest,
    WriteFileRecordRequest, WriteMultipleCoilsRequest, WriteMultipleRegistersRequest,
    WriteSingleCoilRequest, WriteSingleRegisterRequest,
};
use crate::response::{
    DiagnosticsResponse, EncapsulatedInterfaceResponse, ExceptionResponse,
    GetCommEventCounterResponse, GetCommEventLogResponse, MaskWriteRegisterResponse,
    ReadCoilsResponse, ReadDiscreteInputsResponse, ReadExceptionStatusResponse,
    ReadFifoQueueResponse, ReadFileRecordResponse, ReadHoldingRegistersResponse,
    ReadInputRegistersResponse, ReadWriteMultipleRegistersResponse, ReportServerIdResponse,
    WriteFileRecordResponse, WriteMultipleCoilsResponse, WriteMultipleRegistersResponse,
    WriteSingleCoilResponse, WriteSingleRegisterResponse,
};

/// Split a raw PDU into function code + data without further dispatching.
///
/// The input slice starts at the function code byte (after MBAP header or RTU address).
///
/// # Errors
///
/// Returns [`DecodeError::PduTooLarge`] if `pdu` exceeds the Modbus 253-byte
/// PDU ceiling. Returns [`DecodeError::Truncated`] if the slice is empty.
pub fn decode_pdu_ref(pdu: &[u8]) -> Result<PduRef<'_>, DecodeError> {
    if pdu.len() > MAX_PDU_SIZE {
        return Err(DecodeError::PduTooLarge {
            length: pdu.len(),
            maximum: MAX_PDU_SIZE,
        });
    }
    if pdu.is_empty() {
        return Err(DecodeError::Truncated {
            expected: 1,
            actual: 0,
        });
    }
    Ok(PduRef {
        function_code: pdu[0],
        data: &pdu[1..],
    })
}

/// Decode a raw PDU into a typed request.
///
/// The input slice starts at the function code byte.
///
/// # Errors
///
/// Returns a [`DecodeError`] if the PDU is malformed or the function code is unknown.
pub fn decode_request(pdu: &[u8]) -> Result<RequestPdu<'_>, DecodeError> {
    let pdu_ref = decode_pdu_ref(pdu)?;
    let fc = pdu_ref.function_code;
    let data = pdu_ref.data;

    let fc_enum = FunctionCode::from_raw(fc).ok_or(DecodeError::UnknownFunctionCode(fc))?;

    match fc_enum {
        FunctionCode::ReadCoils => ReadCoilsRequest::decode(data).map(RequestPdu::ReadCoils),
        FunctionCode::ReadDiscreteInputs => {
            ReadDiscreteInputsRequest::decode(data).map(RequestPdu::ReadDiscreteInputs)
        }
        FunctionCode::ReadHoldingRegisters => {
            ReadHoldingRegistersRequest::decode(data).map(RequestPdu::ReadHoldingRegisters)
        }
        FunctionCode::ReadInputRegisters => {
            ReadInputRegistersRequest::decode(data).map(RequestPdu::ReadInputRegisters)
        }
        FunctionCode::WriteSingleCoil => {
            WriteSingleCoilRequest::decode(data).map(RequestPdu::WriteSingleCoil)
        }
        FunctionCode::WriteSingleRegister => {
            WriteSingleRegisterRequest::decode(data).map(RequestPdu::WriteSingleRegister)
        }
        FunctionCode::ReadExceptionStatus => {
            DecodeError::check_exact_len(data, 0)?;
            Ok(RequestPdu::ReadExceptionStatus)
        }
        FunctionCode::Diagnostics => DiagnosticsRequest::decode(data).map(RequestPdu::Diagnostics),
        FunctionCode::GetCommEventCounter => {
            DecodeError::check_exact_len(data, 0)?;
            Ok(RequestPdu::GetCommEventCounter)
        }
        FunctionCode::GetCommEventLog => {
            DecodeError::check_exact_len(data, 0)?;
            Ok(RequestPdu::GetCommEventLog)
        }
        FunctionCode::WriteMultipleCoils => {
            WriteMultipleCoilsRequest::decode(data).map(RequestPdu::WriteMultipleCoils)
        }
        FunctionCode::WriteMultipleRegisters => {
            WriteMultipleRegistersRequest::decode(data).map(RequestPdu::WriteMultipleRegisters)
        }
        FunctionCode::ReportServerId => {
            DecodeError::check_exact_len(data, 0)?;
            Ok(RequestPdu::ReportServerId)
        }
        FunctionCode::ReadFileRecord => {
            ReadFileRecordRequest::decode(data).map(RequestPdu::ReadFileRecord)
        }
        FunctionCode::WriteFileRecord => {
            WriteFileRecordRequest::decode(data).map(RequestPdu::WriteFileRecord)
        }
        FunctionCode::MaskWriteRegister => {
            MaskWriteRegisterRequest::decode(data).map(RequestPdu::MaskWriteRegister)
        }
        FunctionCode::ReadWriteMultipleRegisters => ReadWriteMultipleRegistersRequest::decode(data)
            .map(RequestPdu::ReadWriteMultipleRegisters),
        FunctionCode::ReadFifoQueue => {
            ReadFifoQueueRequest::decode(data).map(RequestPdu::ReadFifoQueue)
        }
        FunctionCode::EncapsulatedInterfaceTransport => {
            EncapsulatedInterfaceRequest::decode(data).map(RequestPdu::EncapsulatedInterface)
        }
        FunctionCode::Custom(fc) => Ok(RequestPdu::Custom(fc, data)),
    }
}

/// Decode a raw PDU into a typed response (or exception response).
///
/// The input slice starts at the function code byte.
///
/// # Errors
///
/// Returns a [`DecodeError`] if the PDU is malformed or the function code is unknown.
pub fn decode_response(pdu: &[u8]) -> Result<ResponsePdu<'_>, DecodeError> {
    let pdu_ref = decode_pdu_ref(pdu)?;
    let fc = pdu_ref.function_code;
    let data = pdu_ref.data;

    // Check for exception response (MSB set).
    if FunctionCode::is_exception_response(fc) {
        return ExceptionResponse::decode(fc, data).map(ResponsePdu::Exception);
    }

    // Exception-flagged bytes handled above. from_raw returns named variants for
    // public codes and Custom for nonzero vendor codes.
    let fc_enum = FunctionCode::from_raw(fc).ok_or(DecodeError::UnknownFunctionCode(fc))?;

    match fc_enum {
        FunctionCode::ReadCoils => ReadCoilsResponse::decode(data).map(ResponsePdu::ReadCoils),
        FunctionCode::ReadDiscreteInputs => {
            ReadDiscreteInputsResponse::decode(data).map(ResponsePdu::ReadDiscreteInputs)
        }
        FunctionCode::ReadHoldingRegisters => {
            ReadHoldingRegistersResponse::decode(data).map(ResponsePdu::ReadHoldingRegisters)
        }
        FunctionCode::ReadInputRegisters => {
            ReadInputRegistersResponse::decode(data).map(ResponsePdu::ReadInputRegisters)
        }
        FunctionCode::WriteSingleCoil => {
            WriteSingleCoilResponse::decode(data).map(ResponsePdu::WriteSingleCoil)
        }
        FunctionCode::WriteSingleRegister => {
            WriteSingleRegisterResponse::decode(data).map(ResponsePdu::WriteSingleRegister)
        }
        FunctionCode::ReadExceptionStatus => {
            ReadExceptionStatusResponse::decode(data).map(ResponsePdu::ReadExceptionStatus)
        }
        FunctionCode::Diagnostics => {
            DiagnosticsResponse::decode(data).map(ResponsePdu::Diagnostics)
        }
        FunctionCode::GetCommEventCounter => {
            GetCommEventCounterResponse::decode(data).map(ResponsePdu::GetCommEventCounter)
        }
        FunctionCode::GetCommEventLog => {
            GetCommEventLogResponse::decode(data).map(ResponsePdu::GetCommEventLog)
        }
        FunctionCode::WriteMultipleCoils => {
            WriteMultipleCoilsResponse::decode(data).map(ResponsePdu::WriteMultipleCoils)
        }
        FunctionCode::WriteMultipleRegisters => {
            WriteMultipleRegistersResponse::decode(data).map(ResponsePdu::WriteMultipleRegisters)
        }
        FunctionCode::ReportServerId => {
            ReportServerIdResponse::decode(data).map(ResponsePdu::ReportServerId)
        }
        FunctionCode::ReadFileRecord => {
            ReadFileRecordResponse::decode(data).map(ResponsePdu::ReadFileRecord)
        }
        FunctionCode::WriteFileRecord => {
            WriteFileRecordResponse::decode(data).map(ResponsePdu::WriteFileRecord)
        }
        FunctionCode::MaskWriteRegister => {
            MaskWriteRegisterResponse::decode(data).map(ResponsePdu::MaskWriteRegister)
        }
        FunctionCode::ReadWriteMultipleRegisters => {
            ReadWriteMultipleRegistersResponse::decode(data)
                .map(ResponsePdu::ReadWriteMultipleRegisters)
        }
        FunctionCode::ReadFifoQueue => {
            ReadFifoQueueResponse::decode(data).map(ResponsePdu::ReadFifoQueue)
        }
        FunctionCode::EncapsulatedInterfaceTransport => {
            EncapsulatedInterfaceResponse::decode(data).map(ResponsePdu::EncapsulatedInterface)
        }
        FunctionCode::Custom(fc) => Ok(ResponsePdu::Custom(fc, data)),
    }
}
