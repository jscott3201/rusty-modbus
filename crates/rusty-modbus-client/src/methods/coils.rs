//! Coil and discrete input access methods — FC 01, 02, 05, 0F.

use rusty_modbus_codec::request::{
    ReadCoilsRequest, ReadDiscreteInputsRequest, WriteMultipleCoilsRequest, WriteSingleCoilRequest,
};
use rusty_modbus_frame::OwnedResponsePdu;
use rusty_modbus_types::{Address, CoilValue, FunctionCode, Quantity, UnitId};

use rusty_modbus_tcp::transport::TransportSink;

use crate::client::ModbusClient;
use crate::error::ClientError;
use crate::methods::{
    MAX_WRITE_MULTIPLE_COILS, checked_byte_count_len, checked_quantity_len, encode_request,
};

impl<S: TransportSink + Send + 'static> ModbusClient<S> {
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
        let len = encode_request(&req, &mut buf)?;

        let response = self
            .send_with_retry(unit_id, FunctionCode::ReadCoils, &buf[..len])
            .await?;

        match response {
            OwnedResponsePdu::ReadCoils(rc) => {
                // Guard against a server returning fewer bytes than the
                // requested quantity needs: `coil(i)` indexes the bit buffer
                // unchecked, so a short response would otherwise panic.
                let needed = (quantity as usize).div_ceil(8);
                if usize::from(rc.byte_count) < needed {
                    return Err(ClientError::ShortResponse {
                        expected: needed,
                        actual: usize::from(rc.byte_count),
                    });
                }
                let mut coils = Vec::with_capacity(quantity as usize);
                for i in 0..quantity as usize {
                    coils.push(rc.coil(i));
                }
                Ok(coils)
            }
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(
                rusty_modbus_codec::DecodeError::UnknownFunctionCode(0),
            )),
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
        let len = encode_request(&req, &mut buf)?;

        let response = self
            .send_with_retry(unit_id, FunctionCode::ReadDiscreteInputs, &buf[..len])
            .await?;

        match response {
            OwnedResponsePdu::ReadDiscreteInputs(rd) => {
                // Guard against a short response (see `read_coils`): `coil(i)`
                // indexes the bit buffer unchecked.
                let needed = (quantity as usize).div_ceil(8);
                if usize::from(rd.byte_count) < needed {
                    return Err(ClientError::ShortResponse {
                        expected: needed,
                        actual: usize::from(rd.byte_count),
                    });
                }
                let mut inputs = Vec::with_capacity(quantity as usize);
                for i in 0..quantity as usize {
                    inputs.push(rd.coil(i));
                }
                Ok(inputs)
            }
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(
                rusty_modbus_codec::DecodeError::UnknownFunctionCode(0),
            )),
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
        let len = encode_request(&req, &mut buf)?;

        if unit_id.is_broadcast() {
            return self.send_broadcast(&buf[..len]).await;
        }

        let response = self
            .send_with_retry(unit_id, FunctionCode::WriteSingleCoil, &buf[..len])
            .await?;

        match response {
            OwnedResponsePdu::WriteSingleCoil(resp) => {
                expect_echo("address", address, resp.address.0)?;
                expect_echo("value", req.value.to_wire(), resp.value.to_wire())
            }
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(
                rusty_modbus_codec::DecodeError::UnknownFunctionCode(0),
            )),
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
        let quantity = checked_quantity_len(values.len(), MAX_WRITE_MULTIPLE_COILS)?;
        let byte_count = checked_byte_count_len(values.len().div_ceil(8))?;
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
        let len = encode_request(&req, &mut buf)?;

        if unit_id.is_broadcast() {
            return self.send_broadcast(&buf[..len]).await;
        }

        let response = self
            .send_with_retry(unit_id, FunctionCode::WriteMultipleCoils, &buf[..len])
            .await?;

        match response {
            OwnedResponsePdu::WriteMultipleCoils(resp) => {
                expect_echo("address", address, resp.address.0)?;
                expect_echo("quantity", quantity, resp.quantity.0)
            }
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(
                rusty_modbus_codec::DecodeError::UnknownFunctionCode(0),
            )),
        }
    }
}

fn expect_echo(field: &'static str, expected: u16, got: u16) -> Result<(), ClientError> {
    if expected == got {
        Ok(())
    } else {
        Err(ClientError::UnexpectedResponseEcho {
            field,
            expected,
            got,
        })
    }
}
