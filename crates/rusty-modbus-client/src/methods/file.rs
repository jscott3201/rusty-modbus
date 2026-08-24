//! File record access methods — FC 0x14, 0x15.

use rusty_modbus_codec::request::{ReadFileRecordRequest, WriteFileRecordRequest};
use rusty_modbus_frame::OwnedResponsePdu;
use rusty_modbus_frame::owned::{OwnedReadFileRecordResponse, OwnedWriteFileRecordResponse};
use rusty_modbus_types::{FunctionCode, UnitId};

use rusty_modbus_tcp::transport::TransportSink;

use crate::client::{ModbusClient, RequestKind};
use crate::error::ClientError;
use crate::methods::{checked_file_record_byte_count, encode_request};

impl<S: TransportSink + Send + 'static> ModbusClient<S> {
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
            byte_count: checked_file_record_byte_count(sub_request_data.len())?,
            sub_requests: sub_request_data,
        };

        let mut buf = [0u8; 256];
        let len = encode_request(&req, &mut buf)?;

        let response = self
            .send_with_retry(
                unit_id,
                FunctionCode::ReadFileRecord,
                &buf[..len],
                RequestKind::ReplaySafe,
            )
            .await?;

        match response {
            OwnedResponsePdu::ReadFileRecord(r) => Ok(r),
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(
                rusty_modbus_codec::DecodeError::UnknownFunctionCode(0),
            )),
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
            byte_count: checked_file_record_byte_count(sub_request_data.len())?,
            sub_requests: sub_request_data,
        };

        let mut buf = [0u8; 256];
        let len = encode_request(&req, &mut buf)?;

        let response = self
            .send_with_retry(
                unit_id,
                FunctionCode::WriteFileRecord,
                &buf[..len],
                RequestKind::Mutating,
            )
            .await?;

        match response {
            OwnedResponsePdu::WriteFileRecord(r) => Ok(r),
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(
                rusty_modbus_codec::DecodeError::UnknownFunctionCode(0),
            )),
        }
    }
}
