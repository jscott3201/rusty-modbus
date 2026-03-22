//! File record access methods — FC 0x14, 0x15.

use rusty_modbus_codec::request::{Encode, ReadFileRecordRequest, WriteFileRecordRequest};
use rusty_modbus_frame::owned::{OwnedReadFileRecordResponse, OwnedWriteFileRecordResponse};
use rusty_modbus_frame::OwnedResponsePdu;
use rusty_modbus_types::{FunctionCode, UnitId};

use crate::client::ModbusClient;
use crate::error::ClientError;

impl ModbusClient {
    /// Read file records (FC 0x14).
    ///
    /// `sub_request_data` contains the raw sub-request bytes. Each sub-request
    /// is 7 bytes: `reference_type` (`0x06`) + `file_number` (2) + `record_number` (2)
    /// + `record_length` (2).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on timeout, transport failure, or Modbus exception.
    pub async fn read_file_record(
        &self,
        unit_id: UnitId,
        sub_request_data: &[u8],
    ) -> Result<OwnedReadFileRecordResponse, ClientError> {
        if unit_id.is_broadcast() {
            return Err(ClientError::BroadcastReadNotAllowed);
        }

        let req = ReadFileRecordRequest {
            byte_count: u8::try_from(sub_request_data.len()).unwrap_or(u8::MAX),
            sub_requests: sub_request_data,
        };

        let mut buf = [0u8; 256];
        let len = req.encode_into(&mut buf).map_err(|_| ClientError::Codec(
            rusty_modbus_codec::DecodeError::Truncated { expected: 1, actual: 0 },
        ))?;

        let response = self.send_with_retry(
            unit_id, FunctionCode::ReadFileRecord, &buf[..len],
        ).await?;

        match response {
            OwnedResponsePdu::ReadFileRecord(r) => Ok(r),
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(rusty_modbus_codec::DecodeError::UnknownFunctionCode(0))),
        }
    }

    /// Write file records (FC 0x15).
    ///
    /// `sub_request_data` contains the raw sub-request bytes. Each sub-request
    /// contains: `reference_type` (`0x06`) + `file_number` (2) + `record_number` (2)
    /// + `record_length` (2) + `record_data` (N bytes).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on timeout, transport failure, or Modbus exception.
    pub async fn write_file_record(
        &self,
        unit_id: UnitId,
        sub_request_data: &[u8],
    ) -> Result<OwnedWriteFileRecordResponse, ClientError> {
        if unit_id.is_broadcast() {
            return Err(ClientError::BroadcastReadNotAllowed);
        }

        let req = WriteFileRecordRequest {
            byte_count: u8::try_from(sub_request_data.len()).unwrap_or(u8::MAX),
            sub_requests: sub_request_data,
        };

        let mut buf = [0u8; 256];
        let len = req.encode_into(&mut buf).map_err(|_| ClientError::Codec(
            rusty_modbus_codec::DecodeError::Truncated { expected: 1, actual: 0 },
        ))?;

        let response = self.send_with_retry(
            unit_id, FunctionCode::WriteFileRecord, &buf[..len],
        ).await?;

        match response {
            OwnedResponsePdu::WriteFileRecord(r) => Ok(r),
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(rusty_modbus_codec::DecodeError::UnknownFunctionCode(0))),
        }
    }
}
