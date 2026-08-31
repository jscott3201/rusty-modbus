//! Device identification method — FC 0x2B, MEI 0x0E.

use rusty_modbus_codec::request::ReadDeviceIdentificationRequest;
use rusty_modbus_codec::response::device_id::ReadDeviceIdentificationResponse;
use rusty_modbus_frame::OwnedDeviceIdentification;
use rusty_modbus_frame::OwnedResponsePdu;
use rusty_modbus_types::{DeviceIdCode, FunctionCode, UnitId};

use rusty_modbus_tcp::transport::TransportSink;

use crate::client::{ModbusClient, RequestKind};
use crate::error::ClientError;
use crate::methods::encode_request;

const MAX_BASIC_DEVICE_ID_RESPONSE_PAGES: u8 = 3;

impl<S: TransportSink + Send + 'static> ModbusClient<S> {
    /// Read Device Identification (FC 0x2B / MEI 0x0E).
    ///
    /// Sends a `BasicStream` request and collects continuation responses
    /// if `more_follows` is set. Returns the basic identification objects.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on timeout, transport failure, or Modbus exception.
    pub async fn read_device_identification(
        &self,
        unit_id: UnitId,
    ) -> Result<OwnedDeviceIdentification, ClientError> {
        if unit_id.is_broadcast() {
            return Err(ClientError::BroadcastReadNotAllowed);
        }

        let mut result = OwnedDeviceIdentification::default();
        let mut next_object_id: u8 = 0x00;
        let mut response_pages: u8 = 0;

        loop {
            let request_object_id = next_object_id;
            let req = ReadDeviceIdentificationRequest {
                device_id_code: DeviceIdCode::BasicStream,
                object_id: request_object_id,
            };
            let mut buf = [0u8; 4];
            let len = encode_request(&req, &mut buf)?;

            let response = self
                .send_with_retry(
                    unit_id,
                    FunctionCode::EncapsulatedInterfaceTransport,
                    &buf[..len],
                    RequestKind::ReplaySafe,
                )
                .await?;

            let pdu_data = self.finish_typed_response(match response {
                OwnedResponsePdu::EncapsulatedInterface(mei) => Ok(mei.data),
                OwnedResponsePdu::Exception(exc) => Err(ClientError::Exception(exc)),
                _ => Err(ClientError::Codec(
                    rusty_modbus_codec::DecodeError::UnknownFunctionCode(0),
                )),
            })?;

            // OwnedEncapsulatedInterfaceResponse.data starts after FC+MEI bytes.
            // The codec decoder expects data starting at the MEI type byte.
            // Prepend the MEI type byte to reconstruct the expected layout.
            let mut decode_buf = Vec::with_capacity(1 + pdu_data.len());
            decode_buf.push(0x0E);
            decode_buf.extend_from_slice(&pdu_data);

            let dev_id = self.finish_typed_response(
                ReadDeviceIdentificationResponse::decode(&decode_buf).map_err(ClientError::Codec),
            )?;
            response_pages += 1;

            for obj in dev_id.objects() {
                let value = String::from_utf8_lossy(obj.value).into_owned();
                match obj.id {
                    0x00 => result.vendor_name = Some(value),
                    0x01 => result.product_code = Some(value),
                    0x02 => result.major_minor_revision = Some(value),
                    _ => {} // ignore higher objects for basic request
                }
            }

            if !dev_id.more_follows {
                break;
            }
            if response_pages >= MAX_BASIC_DEVICE_ID_RESPONSE_PAGES {
                return self.finish_typed_response(Err(
                    ClientError::DeviceIdentificationPaginationLimit {
                        limit: MAX_BASIC_DEVICE_ID_RESPONSE_PAGES,
                    },
                ));
            }
            if dev_id.next_object_id <= request_object_id {
                return self.finish_typed_response(Err(
                    ClientError::InvalidDeviceIdentificationContinuation {
                        previous_object_id: request_object_id,
                        next_object_id: dev_id.next_object_id,
                    },
                ));
            }
            next_object_id = dev_id.next_object_id;
        }

        Ok(result)
    }
}
