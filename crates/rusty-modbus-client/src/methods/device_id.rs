//! Device identification method — FC 0x2B, MEI 0x0E.

use rusty_modbus_codec::request::ReadDeviceIdentificationRequest;
use rusty_modbus_codec::response::device_id::ReadDeviceIdentificationResponse;
use rusty_modbus_frame::OwnedDeviceIdentification;
use rusty_modbus_frame::OwnedResponsePdu;
use rusty_modbus_types::{DeviceIdCode, FunctionCode, UnitId};

use rusty_modbus_tcp::transport::TransportSink;

use crate::client::ModbusClient;
use crate::error::ClientError;
use crate::methods::encode_request;

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
        let mut result = OwnedDeviceIdentification::default();
        let mut next_object_id: u8 = 0x00;

        loop {
            let req = ReadDeviceIdentificationRequest {
                device_id_code: DeviceIdCode::BasicStream,
                object_id: next_object_id,
            };
            let mut buf = [0u8; 4];
            let len = encode_request(&req, &mut buf)?;

            let response = self
                .send_with_retry(
                    unit_id,
                    FunctionCode::EncapsulatedInterfaceTransport,
                    &buf[..len],
                )
                .await?;

            let pdu_data = match response {
                OwnedResponsePdu::EncapsulatedInterface(mei) => mei.data,
                OwnedResponsePdu::Exception(exc) => return Err(ClientError::Exception(exc)),
                _ => {
                    return Err(ClientError::Codec(
                        rusty_modbus_codec::DecodeError::UnknownFunctionCode(0),
                    ));
                }
            };

            // OwnedEncapsulatedInterfaceResponse.data starts after FC+MEI bytes.
            // The codec decoder expects data starting at the MEI type byte.
            // Prepend the MEI type byte to reconstruct the expected layout.
            let mut decode_buf = Vec::with_capacity(1 + pdu_data.len());
            decode_buf.push(0x0E);
            decode_buf.extend_from_slice(&pdu_data);

            let dev_id = ReadDeviceIdentificationResponse::decode(&decode_buf)
                .map_err(ClientError::Codec)?;

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
            next_object_id = dev_id.next_object_id;
        }

        Ok(result)
    }
}
