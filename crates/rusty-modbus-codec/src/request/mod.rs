//! Modbus request PDU types — decode and encode for every public function code.

mod bit_read;
mod bit_write;
pub mod device_id;
mod diagnostic;
mod fifo;
mod file;
mod mei;
mod reg_read;
mod reg_write;

pub use bit_read::{ReadCoilsRequest, ReadDiscreteInputsRequest};
pub use bit_write::{WriteMultipleCoilsRequest, WriteSingleCoilRequest};
pub use device_id::ReadDeviceIdentificationRequest;
pub use diagnostic::DiagnosticsRequest;
pub use fifo::ReadFifoQueueRequest;
pub use file::{FileSubRequest, ReadFileRecordRequest, WriteFileRecordRequest};
pub use mei::EncapsulatedInterfaceRequest;
pub use reg_read::{ReadHoldingRegistersRequest, ReadInputRegistersRequest};
pub use reg_write::{
    MaskWriteRegisterRequest, ReadWriteMultipleRegistersRequest, WriteMultipleRegistersRequest,
    WriteSingleRegisterRequest,
};

use crate::error::EncodeError;

/// Trait for encoding a Modbus PDU into a byte buffer.
pub trait Encode {
    /// Write the full PDU (function code + data) into `buf`.
    ///
    /// Returns the number of bytes written on success.
    ///
    /// # Errors
    ///
    /// Returns [`EncodeError::BufferTooSmall`] if `buf` is shorter than
    /// [`encoded_len`](Self::encoded_len). Returns [`EncodeError::PduTooLarge`]
    /// if the encoded PDU would exceed the Modbus 253-byte ceiling.
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError>;

    /// Total encoded length in bytes (including the function code byte).
    fn encoded_len(&self) -> usize;
}
