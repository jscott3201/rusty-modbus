//! Register access methods — FC 03, 04, 06, 10, 16, 17, 18.

use bytes::Bytes;
use rusty_modbus_codec::request::{
    MaskWriteRegisterRequest, ReadHoldingRegistersRequest, ReadInputRegistersRequest,
    ReadWriteMultipleRegistersRequest, WriteMultipleRegistersRequest, WriteSingleRegisterRequest,
};
use rusty_modbus_frame::OwnedResponsePdu;
use rusty_modbus_types::{Address, FunctionCode, Quantity, UnitId};

use rusty_modbus_tcp::transport::TransportSink;

use crate::client::ModbusClient;
use crate::error::ClientError;
use crate::methods::{
    MAX_READ_WRITE_MULTIPLE_WRITE_REGISTERS, MAX_WRITE_MULTIPLE_REGISTERS, checked_byte_count_len,
    checked_quantity_len, encode_request,
};

impl<S: TransportSink + Send + 'static> ModbusClient<S> {
    /// Read holding registers (FC 0x03).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on timeout, transport failure, or Modbus exception.
    pub async fn read_holding_registers(
        &self,
        unit_id: UnitId,
        address: u16,
        quantity: u16,
    ) -> Result<Vec<u16>, ClientError> {
        if unit_id.is_broadcast() {
            return Err(ClientError::BroadcastReadNotAllowed);
        }

        let req = ReadHoldingRegistersRequest {
            address: Address(address),
            quantity: Quantity(quantity),
        };
        let mut buf = [0u8; 5];
        let len = encode_request(&req, &mut buf)?;

        let response = self
            .send_with_retry(unit_id, FunctionCode::ReadHoldingRegisters, &buf[..len])
            .await?;

        match response {
            OwnedResponsePdu::ReadHoldingRegisters(rhr) => {
                let needed = quantity as usize * 2;
                if usize::from(rhr.byte_count) < needed {
                    return Err(ClientError::ShortResponse {
                        expected: needed,
                        actual: usize::from(rhr.byte_count),
                    });
                }
                Ok(rhr.registers().take(quantity as usize).collect())
            }
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(
                rusty_modbus_codec::DecodeError::UnknownFunctionCode(0),
            )),
        }
    }

    /// Read holding registers returning raw bytes (FC 0x03, zero-copy variant).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on timeout, transport failure, or Modbus exception.
    pub async fn read_holding_registers_raw(
        &self,
        unit_id: UnitId,
        address: u16,
        quantity: u16,
    ) -> Result<Bytes, ClientError> {
        if unit_id.is_broadcast() {
            return Err(ClientError::BroadcastReadNotAllowed);
        }

        let req = ReadHoldingRegistersRequest {
            address: Address(address),
            quantity: Quantity(quantity),
        };
        let mut buf = [0u8; 5];
        let len = encode_request(&req, &mut buf)?;

        let response = self
            .send_with_retry(unit_id, FunctionCode::ReadHoldingRegisters, &buf[..len])
            .await?;

        match response {
            OwnedResponsePdu::ReadHoldingRegisters(rhr) => {
                let needed = quantity as usize * 2;
                if usize::from(rhr.byte_count) < needed {
                    return Err(ClientError::ShortResponse {
                        expected: needed,
                        actual: usize::from(rhr.byte_count),
                    });
                }
                Ok(Bytes::copy_from_slice(rhr.raw()))
            }
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(
                rusty_modbus_codec::DecodeError::UnknownFunctionCode(0),
            )),
        }
    }

    /// Read input registers (FC 0x04).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on timeout, transport failure, or Modbus exception.
    pub async fn read_input_registers(
        &self,
        unit_id: UnitId,
        address: u16,
        quantity: u16,
    ) -> Result<Vec<u16>, ClientError> {
        if unit_id.is_broadcast() {
            return Err(ClientError::BroadcastReadNotAllowed);
        }

        let req = ReadInputRegistersRequest {
            address: Address(address),
            quantity: Quantity(quantity),
        };
        let mut buf = [0u8; 5];
        let len = encode_request(&req, &mut buf)?;

        let response = self
            .send_with_retry(unit_id, FunctionCode::ReadInputRegisters, &buf[..len])
            .await?;

        match response {
            OwnedResponsePdu::ReadInputRegisters(rir) => {
                let needed = quantity as usize * 2;
                if usize::from(rir.byte_count) < needed {
                    return Err(ClientError::ShortResponse {
                        expected: needed,
                        actual: usize::from(rir.byte_count),
                    });
                }
                Ok(rir.registers().take(quantity as usize).collect())
            }
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(
                rusty_modbus_codec::DecodeError::UnknownFunctionCode(0),
            )),
        }
    }

    /// Write a single register (FC 0x06).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on timeout, transport failure, or Modbus exception.
    pub async fn write_single_register(
        &self,
        unit_id: UnitId,
        address: u16,
        value: u16,
    ) -> Result<(), ClientError> {
        let req = WriteSingleRegisterRequest {
            address: Address(address),
            value,
        };
        let mut buf = [0u8; 5];
        let len = encode_request(&req, &mut buf)?;

        if unit_id.is_broadcast() {
            return self.send_broadcast(&buf[..len]).await;
        }

        let response = self
            .send_with_retry(unit_id, FunctionCode::WriteSingleRegister, &buf[..len])
            .await?;

        match response {
            OwnedResponsePdu::WriteSingleRegister(_) => Ok(()),
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(
                rusty_modbus_codec::DecodeError::UnknownFunctionCode(0),
            )),
        }
    }

    /// Write multiple registers (FC 0x10).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on timeout, transport failure, or Modbus exception.
    pub async fn write_multiple_registers(
        &self,
        unit_id: UnitId,
        address: u16,
        values: &[u16],
    ) -> Result<(), ClientError> {
        let quantity = checked_quantity_len(values.len(), MAX_WRITE_MULTIPLE_REGISTERS)?;
        let byte_count = checked_byte_count_len(values.len() * 2)?;
        let mut value_bytes = vec![0u8; values.len() * 2];
        for (i, &v) in values.iter().enumerate() {
            value_bytes[i * 2..i * 2 + 2].copy_from_slice(&v.to_be_bytes());
        }

        let req = WriteMultipleRegistersRequest {
            address: Address(address),
            quantity: Quantity(quantity),
            byte_count,
            register_values: &value_bytes,
        };

        let mut buf = [0u8; 256];
        let len = encode_request(&req, &mut buf)?;

        if unit_id.is_broadcast() {
            return self.send_broadcast(&buf[..len]).await;
        }

        let response = self
            .send_with_retry(unit_id, FunctionCode::WriteMultipleRegisters, &buf[..len])
            .await?;

        match response {
            OwnedResponsePdu::WriteMultipleRegisters(_) => Ok(()),
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(
                rusty_modbus_codec::DecodeError::UnknownFunctionCode(0),
            )),
        }
    }

    /// Mask write register (FC 0x16).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on timeout, transport failure, or Modbus exception.
    pub async fn mask_write_register(
        &self,
        unit_id: UnitId,
        address: u16,
        and_mask: u16,
        or_mask: u16,
    ) -> Result<(), ClientError> {
        let req = MaskWriteRegisterRequest {
            address: Address(address),
            and_mask,
            or_mask,
        };
        let mut buf = [0u8; 7];
        let len = encode_request(&req, &mut buf)?;

        if unit_id.is_broadcast() {
            return self.send_broadcast(&buf[..len]).await;
        }

        let response = self
            .send_with_retry(unit_id, FunctionCode::MaskWriteRegister, &buf[..len])
            .await?;

        match response {
            OwnedResponsePdu::MaskWriteRegister(_) => Ok(()),
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(
                rusty_modbus_codec::DecodeError::UnknownFunctionCode(0),
            )),
        }
    }

    /// Read and write multiple registers simultaneously (FC 0x17).
    ///
    /// The write is executed before the read per Spec §6.17.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on timeout, transport failure, or Modbus exception.
    pub async fn read_write_multiple_registers(
        &self,
        unit_id: UnitId,
        read_address: u16,
        read_quantity: u16,
        write_address: u16,
        write_values: &[u16],
    ) -> Result<Vec<u16>, ClientError> {
        if unit_id.is_broadcast() {
            return Err(ClientError::BroadcastReadNotAllowed);
        }

        let write_quantity =
            checked_quantity_len(write_values.len(), MAX_READ_WRITE_MULTIPLE_WRITE_REGISTERS)?;
        let write_byte_count = checked_byte_count_len(write_values.len() * 2)?;
        let mut value_bytes = vec![0u8; write_values.len() * 2];
        for (i, &v) in write_values.iter().enumerate() {
            value_bytes[i * 2..i * 2 + 2].copy_from_slice(&v.to_be_bytes());
        }

        let req = ReadWriteMultipleRegistersRequest {
            read_address: Address(read_address),
            read_quantity: Quantity(read_quantity),
            write_address: Address(write_address),
            write_quantity: Quantity(write_quantity),
            write_byte_count,
            write_register_values: &value_bytes,
        };

        let mut buf = [0u8; 256];
        let len = encode_request(&req, &mut buf)?;

        let response = self
            .send_with_retry(
                unit_id,
                FunctionCode::ReadWriteMultipleRegisters,
                &buf[..len],
            )
            .await?;

        match response {
            OwnedResponsePdu::ReadWriteMultipleRegisters(rw) => {
                let needed = read_quantity as usize * 2;
                if usize::from(rw.byte_count) < needed {
                    return Err(ClientError::ShortResponse {
                        expected: needed,
                        actual: usize::from(rw.byte_count),
                    });
                }
                Ok(rw.registers().take(read_quantity as usize).collect())
            }
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(
                rusty_modbus_codec::DecodeError::UnknownFunctionCode(0),
            )),
        }
    }
}
