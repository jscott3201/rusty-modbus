//! Read Device Identification request (FC 0x2B, MEI type 0x0E, Spec V1.1b3 §6.21).

use rusty_modbus_types::{DeviceIdCode, FunctionCode, MeiType};

use crate::error::{DecodeError, EncodeError};
use crate::request::Encode;

/// FC 0x2B / MEI 0x0E — Read Device Identification request.
///
/// Spec V1.1b3 §6.21. Wire format: FC(1) + MEI(1) + DevIdCode(1) + ObjectId(1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadDeviceIdentificationRequest {
    /// Access code: basic (0x01), regular (0x02), extended (0x03), or individual (0x04).
    pub device_id_code: DeviceIdCode,
    /// First object ID to read (0x00 for stream access from the beginning).
    pub object_id: u8,
}

impl ReadDeviceIdentificationRequest {
    /// Decode from the data bytes following the function code.
    ///
    /// The `data` slice starts at the MEI type byte (FC byte already consumed).
    ///
    /// # Errors
    ///
    /// Returns `DecodeError::Truncated` if data is too short.
    /// Returns `DecodeError::UnknownMeiType` if MEI type is not 0x0E.
    /// Returns `DecodeError::InvalidDeviceIdCode` if the access code is unrecognized.
    pub fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        if data.len() < 3 {
            return Err(DecodeError::Truncated {
                expected: 3,
                actual: data.len(),
            });
        }
        if data[0] != MeiType::ReadDeviceIdentification.code() {
            return Err(DecodeError::UnknownMeiType(data[0]));
        }
        let device_id_code =
            DeviceIdCode::from_raw(data[1]).ok_or(DecodeError::InvalidDeviceIdCode(data[1]))?;
        let object_id = data[2];
        Ok(Self {
            device_id_code,
            object_id,
        })
    }
}

impl Encode for ReadDeviceIdentificationRequest {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        EncodeError::check_pdu_len(len)?;
        buf[0] = FunctionCode::EncapsulatedInterfaceTransport.code();
        buf[1] = MeiType::ReadDeviceIdentification.code();
        buf[2] = self.device_id_code.code();
        buf[3] = self.object_id;
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        4 // FC + MEI + DevIdCode + ObjId
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_basic_stream_request() {
        let req = ReadDeviceIdentificationRequest {
            device_id_code: DeviceIdCode::BasicStream,
            object_id: 0x00,
        };
        let mut buf = [0u8; 5];
        let len = req.encode_into(&mut buf).unwrap();
        assert_eq!(&buf[..len], &[0x2B, 0x0E, 0x01, 0x00]);
    }

    #[test]
    fn decode_basic_stream_request() {
        let data = [0x0E, 0x01, 0x00];
        let req = ReadDeviceIdentificationRequest::decode(&data).unwrap();
        assert_eq!(req.device_id_code, DeviceIdCode::BasicStream);
        assert_eq!(req.object_id, 0x00);
    }

    #[test]
    fn decode_individual_request() {
        let data = [0x0E, 0x04, 0x03];
        let req = ReadDeviceIdentificationRequest::decode(&data).unwrap();
        assert_eq!(req.device_id_code, DeviceIdCode::Individual);
        assert_eq!(req.object_id, 0x03);
    }

    #[test]
    fn decode_wrong_mei_type() {
        let data = [0x0D, 0x01, 0x00];
        assert!(matches!(
            ReadDeviceIdentificationRequest::decode(&data),
            Err(DecodeError::UnknownMeiType(0x0D))
        ));
    }

    #[test]
    fn decode_invalid_device_id_code() {
        let data = [0x0E, 0xFF, 0x00];
        assert!(matches!(
            ReadDeviceIdentificationRequest::decode(&data),
            Err(DecodeError::InvalidDeviceIdCode(0xFF))
        ));
    }

    #[test]
    fn decode_truncated() {
        let data = [0x0E, 0x01];
        assert!(matches!(
            ReadDeviceIdentificationRequest::decode(&data),
            Err(DecodeError::Truncated { .. })
        ));
    }
}
