//! FIFO queue access method — FC 0x18.

use rusty_modbus_codec::request::ReadFifoQueueRequest;
use rusty_modbus_frame::OwnedResponsePdu;
use rusty_modbus_types::{Address, FunctionCode, UnitId};

use rusty_modbus_tcp::transport::TransportSink;

use crate::client::ModbusClient;
use crate::error::ClientError;
use crate::methods::encode_request;

impl<S: TransportSink + Send + 'static> ModbusClient<S> {
    /// Read FIFO queue (FC 0x18).
    ///
    /// Returns the register values from the FIFO queue at the given
    /// pointer address. Maximum 31 values per Spec §6.18.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on timeout, transport failure, or Modbus exception.
    pub async fn read_fifo_queue(
        &self,
        unit_id: UnitId,
        pointer_address: u16,
    ) -> Result<Vec<u16>, ClientError> {
        if unit_id.is_broadcast() {
            return Err(ClientError::BroadcastReadNotAllowed);
        }

        let req = ReadFifoQueueRequest {
            fifo_pointer_address: Address(pointer_address),
        };

        let mut buf = [0u8; 3];
        let len = encode_request(&req, &mut buf)?;

        let response = self
            .send_with_retry(unit_id, FunctionCode::ReadFifoQueue, &buf[..len])
            .await?;

        match response {
            OwnedResponsePdu::ReadFifoQueue(fifo) => Ok(fifo
                .fifo_values
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect()),
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(
                rusty_modbus_codec::DecodeError::UnknownFunctionCode(0),
            )),
        }
    }
}
