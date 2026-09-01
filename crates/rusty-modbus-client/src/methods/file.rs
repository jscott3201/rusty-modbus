//! File record access methods — FC 0x14, 0x15.

use rusty_modbus_codec::request::{ReadFileRecordRequest, WriteFileRecordRequest};
use rusty_modbus_frame::OwnedResponsePdu;
use rusty_modbus_frame::owned::{OwnedReadFileRecordResponse, OwnedWriteFileRecordResponse};
use rusty_modbus_types::{FunctionCode, UnitId};

use rusty_modbus_tcp::transport::TransportSink;

use crate::client::{ModbusClient, RequestKind};
use crate::error::ClientError;
use crate::methods::{checked_file_record_byte_count, encode_request};

fn read_file_record_sub_response_count(mut data: &[u8]) -> usize {
    let mut count = 0;
    while let Some((&response_len, rest)) = data.split_first() {
        let Some(remaining) = rest.get(usize::from(response_len)..) else {
            break;
        };
        count += 1;
        data = remaining;
    }
    count
}

impl<S: TransportSink + Send + 'static> ModbusClient<S> {
    /// Read file records (FC 0x14).
    ///
    /// `sub_request_data` contains the raw sub-request bytes. Each sub-request
    /// is 7 bytes: `reference_type` (`0x06`) + `file_number` (2) + `record_number` (2)
    /// + `record_length` (2).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on timeout, transport failure, Modbus exception,
    /// or when a normal response does not contain exactly one FC14 response
    /// group for each validated request sub-group.
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
        let expected_sub_responses = sub_request_data.len() / 7;

        let response = self
            .send_with_retry(
                unit_id,
                FunctionCode::ReadFileRecord,
                &buf[..len],
                RequestKind::ReplaySafe,
            )
            .await?;

        let result = match response {
            OwnedResponsePdu::ReadFileRecord(r) => {
                let actual_sub_responses = read_file_record_sub_response_count(&r.data);
                if actual_sub_responses == expected_sub_responses {
                    Ok(r)
                } else {
                    Err(ClientError::UnexpectedFileRecordSubResponseCount {
                        expected: expected_sub_responses,
                        actual: actual_sub_responses,
                    })
                }
            }
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(
                rusty_modbus_codec::DecodeError::UnknownFunctionCode(0),
            )),
        };
        self.finish_typed_response(result)
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

        let result = match response {
            OwnedResponsePdu::WriteFileRecord(r) => Ok(r),
            OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
            _ => Err(ClientError::Codec(
                rusty_modbus_codec::DecodeError::UnknownFunctionCode(0),
            )),
        };
        self.finish_typed_response(result)
    }
}

#[cfg(test)]
mod tests {
    use super::read_file_record_sub_response_count;

    #[test]
    fn file_record_sub_response_count_stops_safely_at_truncated_group() {
        assert_eq!(read_file_record_sub_response_count(&[0xFF]), 0);
        assert_eq!(
            read_file_record_sub_response_count(&[0x03, 0x06, 0x00, 0x01, 0xFF]),
            1
        );
    }
}
