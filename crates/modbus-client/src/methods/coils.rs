//! Coil and discrete input access methods — FC 01, 02, 05, 0F.

use modbus_codec::request::{
    Encode, ReadCoilsRequest, ReadDiscreteInputsRequest, WriteSingleCoilRequest,
    WriteMultipleCoilsRequest,
};
use modbus_frame::OwnedResponsePdu;
use modbus_types::{Address, CoilValue, FunctionCode, Quantity, UnitId};

use crate::client::ModbusClient;
use crate::error::ClientError;

impl ModbusClient {
    /// Read coils (FC 0x01).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on timeout, transport failure, or Modbus exception.
    pub async fn read_coils(
        &self,
        unit_id: UnitId,
        address: u16,
        quantity: u16,
    ) -> Result<Vec<bool>, ClientError> {
        if unit_id.is_broadcast() {
            return Err(ClientError::BroadcastReadNotAllowed);
        }

        let req = ReadCoilsRequest {
            address: Address(address),
            quantity: Quantity(quantity),
        };
        let mut buf = [0u8; 5];
        let len = req.encode_into(&mut buf).map_err(|_| ClientError::Codec(
            modbus_codec::DecodeError::Truncated { expected: 5, actual: 0 },
        ))?;

        let response = self.send_with_retry(unit_id, FunctionCode::ReadCoils, &buf[..len]).await?;

        match response {
            OwnedResponsePdu::ReadCoils(rc) => {
                let mut coils = Vec::with_capacity(quantity as usize);
                for i in 0..quantity as usize {
                    coils.push(rc.coil(i));
                }
                Ok(coils)
            }
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(modbus_codec::DecodeError::UnknownFunctionCode(0))),
        }
    }

    /// Read discrete inputs (FC 0x02).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on timeout, transport failure, or Modbus exception.
    pub async fn read_discrete_inputs(
        &self,
        unit_id: UnitId,
        address: u16,
        quantity: u16,
    ) -> Result<Vec<bool>, ClientError> {
        if unit_id.is_broadcast() {
            return Err(ClientError::BroadcastReadNotAllowed);
        }

        let req = ReadDiscreteInputsRequest {
            address: Address(address),
            quantity: Quantity(quantity),
        };
        let mut buf = [0u8; 5];
        let len = req.encode_into(&mut buf).map_err(|_| ClientError::Codec(
            modbus_codec::DecodeError::Truncated { expected: 5, actual: 0 },
        ))?;

        let response = self.send_with_retry(unit_id, FunctionCode::ReadDiscreteInputs, &buf[..len]).await?;

        match response {
            OwnedResponsePdu::ReadDiscreteInputs(rd) => {
                let mut inputs = Vec::with_capacity(quantity as usize);
                for i in 0..quantity as usize {
                    inputs.push(rd.coil(i));
                }
                Ok(inputs)
            }
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(modbus_codec::DecodeError::UnknownFunctionCode(0))),
        }
    }

    /// Write a single coil (FC 0x05).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on timeout, transport failure, or Modbus exception.
    pub async fn write_single_coil(
        &self,
        unit_id: UnitId,
        address: u16,
        value: bool,
    ) -> Result<(), ClientError> {
        let req = WriteSingleCoilRequest {
            address: Address(address),
            value: CoilValue::from_bool(value),
        };
        let mut buf = [0u8; 5];
        let len = req.encode_into(&mut buf).map_err(|_| ClientError::Codec(
            modbus_codec::DecodeError::Truncated { expected: 5, actual: 0 },
        ))?;

        if unit_id.is_broadcast() {
            return self.send_broadcast(&buf[..len]).await;
        }

        let response = self.send_with_retry(unit_id, FunctionCode::WriteSingleCoil, &buf[..len]).await?;

        match response {
            OwnedResponsePdu::WriteSingleCoil(_) => Ok(()),
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(modbus_codec::DecodeError::UnknownFunctionCode(0))),
        }
    }

    /// Write multiple coils (FC 0x0F).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on timeout, transport failure, or Modbus exception.
    pub async fn write_multiple_coils(
        &self,
        unit_id: UnitId,
        address: u16,
        values: &[bool],
    ) -> Result<(), ClientError> {
        let quantity = u16::try_from(values.len()).unwrap_or(u16::MAX);
        let byte_count = u8::try_from(values.len().div_ceil(8)).unwrap_or(u8::MAX);
        let mut coil_bytes = vec![0u8; byte_count as usize];

        for (i, &val) in values.iter().enumerate() {
            if val {
                coil_bytes[i / 8] |= 1 << (i % 8);
            }
        }

        let req = WriteMultipleCoilsRequest {
            address: Address(address),
            quantity: Quantity(quantity),
            byte_count,
            coil_values: &coil_bytes,
        };

        let mut buf = [0u8; 256];
        let len = req.encode_into(&mut buf).map_err(|_| ClientError::Codec(
            modbus_codec::DecodeError::Truncated { expected: 1, actual: 0 },
        ))?;

        if unit_id.is_broadcast() {
            return self.send_broadcast(&buf[..len]).await;
        }

        let response = self.send_with_retry(unit_id, FunctionCode::WriteMultipleCoils, &buf[..len]).await?;

        match response {
            OwnedResponsePdu::WriteMultipleCoils(_) => Ok(()),
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(modbus_codec::DecodeError::UnknownFunctionCode(0))),
        }
    }
}
