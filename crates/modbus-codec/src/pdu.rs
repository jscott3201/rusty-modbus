//! Core PDU types: `PduRef`, `RequestPdu`, `ResponsePdu`.

use crate::request::{
    DiagnosticsRequest, EncapsulatedInterfaceRequest, MaskWriteRegisterRequest,
    ReadCoilsRequest, ReadDiscreteInputsRequest, ReadFifoQueueRequest, ReadFileRecordRequest,
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

/// Raw decoded PDU — function code + borrowed data slice.
///
/// This is the first thing produced by the decoder before dispatching
/// to typed request/response structs.
#[derive(Debug, Clone, Copy)]
pub struct PduRef<'buf> {
    /// Raw function code byte (may include 0x80 exception flag).
    pub function_code: u8,
    /// Remaining PDU bytes after the function code.
    pub data: &'buf [u8],
}

/// Fully dispatched request PDU. Each variant borrows from the source buffer.
#[derive(Debug)]
pub enum RequestPdu<'buf> {
    /// FC 0x01 — Read Coils.
    ReadCoils(ReadCoilsRequest),
    /// FC 0x02 — Read Discrete Inputs.
    ReadDiscreteInputs(ReadDiscreteInputsRequest),
    /// FC 0x03 — Read Holding Registers.
    ReadHoldingRegisters(ReadHoldingRegistersRequest),
    /// FC 0x04 — Read Input Registers.
    ReadInputRegisters(ReadInputRegistersRequest),
    /// FC 0x05 — Write Single Coil.
    WriteSingleCoil(WriteSingleCoilRequest),
    /// FC 0x06 — Write Single Register.
    WriteSingleRegister(WriteSingleRegisterRequest),
    /// FC 0x07 — Read Exception Status.
    ReadExceptionStatus,
    /// FC 0x08 — Diagnostics.
    Diagnostics(DiagnosticsRequest<'buf>),
    /// FC 0x0B — Get Comm Event Counter.
    GetCommEventCounter,
    /// FC 0x0C — Get Comm Event Log.
    GetCommEventLog,
    /// FC 0x0F — Write Multiple Coils.
    WriteMultipleCoils(WriteMultipleCoilsRequest<'buf>),
    /// FC 0x10 — Write Multiple Registers.
    WriteMultipleRegisters(WriteMultipleRegistersRequest<'buf>),
    /// FC 0x11 — Report Server ID.
    ReportServerId,
    /// FC 0x14 — Read File Record.
    ReadFileRecord(ReadFileRecordRequest<'buf>),
    /// FC 0x15 — Write File Record.
    WriteFileRecord(WriteFileRecordRequest<'buf>),
    /// FC 0x16 — Mask Write Register.
    MaskWriteRegister(MaskWriteRegisterRequest),
    /// FC 0x17 — Read/Write Multiple Registers.
    ReadWriteMultipleRegisters(ReadWriteMultipleRegistersRequest<'buf>),
    /// FC 0x18 — Read FIFO Queue.
    ReadFifoQueue(ReadFifoQueueRequest),
    /// FC 0x2B — Encapsulated Interface Transport.
    EncapsulatedInterface(EncapsulatedInterfaceRequest<'buf>),
    /// Non-standard / vendor-specific request.
    Custom(u8, &'buf [u8]),
}

/// Fully dispatched response PDU. Each variant borrows from the source buffer.
#[derive(Debug)]
pub enum ResponsePdu<'buf> {
    /// FC 0x01 — Read Coils.
    ReadCoils(ReadCoilsResponse<'buf>),
    /// FC 0x02 — Read Discrete Inputs.
    ReadDiscreteInputs(ReadDiscreteInputsResponse<'buf>),
    /// FC 0x03 — Read Holding Registers.
    ReadHoldingRegisters(ReadHoldingRegistersResponse<'buf>),
    /// FC 0x04 — Read Input Registers.
    ReadInputRegisters(ReadInputRegistersResponse<'buf>),
    /// FC 0x05 — Write Single Coil.
    WriteSingleCoil(WriteSingleCoilResponse),
    /// FC 0x06 — Write Single Register.
    WriteSingleRegister(WriteSingleRegisterResponse),
    /// FC 0x07 — Read Exception Status.
    ReadExceptionStatus(ReadExceptionStatusResponse),
    /// FC 0x08 — Diagnostics.
    Diagnostics(DiagnosticsResponse<'buf>),
    /// FC 0x0B — Get Comm Event Counter.
    GetCommEventCounter(GetCommEventCounterResponse),
    /// FC 0x0C — Get Comm Event Log.
    GetCommEventLog(GetCommEventLogResponse<'buf>),
    /// FC 0x0F — Write Multiple Coils.
    WriteMultipleCoils(WriteMultipleCoilsResponse),
    /// FC 0x10 — Write Multiple Registers.
    WriteMultipleRegisters(WriteMultipleRegistersResponse),
    /// FC 0x11 — Report Server ID.
    ReportServerId(ReportServerIdResponse<'buf>),
    /// FC 0x14 — Read File Record.
    ReadFileRecord(ReadFileRecordResponse<'buf>),
    /// FC 0x15 — Write File Record.
    WriteFileRecord(WriteFileRecordResponse<'buf>),
    /// FC 0x16 — Mask Write Register.
    MaskWriteRegister(MaskWriteRegisterResponse),
    /// FC 0x17 — Read/Write Multiple Registers.
    ReadWriteMultipleRegisters(ReadWriteMultipleRegistersResponse<'buf>),
    /// FC 0x18 — Read FIFO Queue.
    ReadFifoQueue(ReadFifoQueueResponse<'buf>),
    /// FC 0x2B — Encapsulated Interface Transport.
    EncapsulatedInterface(EncapsulatedInterfaceResponse<'buf>),
    /// Non-standard / vendor-specific response.
    Custom(u8, &'buf [u8]),
    /// Exception response.
    Exception(ExceptionResponse),
}
