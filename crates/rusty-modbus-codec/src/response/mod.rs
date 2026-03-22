//! Modbus response PDU types — decode and encode for every public function code.

mod bit_read;
mod bit_write;
pub mod device_id;
mod diagnostic;
mod exception;
mod fifo;
mod file;
mod mei;
mod reg_read;
mod reg_write;

pub use bit_read::{ReadCoilsResponse, ReadDiscreteInputsResponse};
pub use bit_write::{WriteMultipleCoilsResponse, WriteSingleCoilResponse};
pub use diagnostic::{
    DiagnosticsResponse, GetCommEventCounterResponse, GetCommEventLogResponse,
    ReadExceptionStatusResponse, ReportServerIdResponse,
};
pub use exception::ExceptionResponse;
pub use fifo::ReadFifoQueueResponse;
pub use file::{ReadFileRecordResponse, WriteFileRecordResponse};
pub use mei::EncapsulatedInterfaceResponse;
pub use reg_read::{ReadHoldingRegistersResponse, ReadInputRegistersResponse};
pub use reg_write::{
    MaskWriteRegisterResponse, ReadWriteMultipleRegistersResponse,
    WriteMultipleRegistersResponse, WriteSingleRegisterResponse,
};
