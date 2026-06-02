//! Read Device Identification response (FC 0x2B, MEI type 0x0E, Spec V1.1b3 §6.21).

use rusty_modbus_types::{DeviceIdCode, MeiType};

use crate::error::DecodeError;

/// A single identification object parsed from the response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceIdObjectEntry<'buf> {
    /// Object ID (0x00–0x06 standard, 0x80–0xFF vendor-specific).
    pub id: u8,
    /// Object value bytes (typically UTF-8 text).
    pub value: &'buf [u8],
}

/// FC 0x2B / MEI 0x0E — Read Device Identification response.
///
/// Spec V1.1b3 §6.21. Object data is borrowed from the source buffer.
/// Use [`objects`](Self::objects) to iterate over individual entries.
#[derive(Debug)]
pub struct ReadDeviceIdentificationResponse<'buf> {
    /// The access code that was requested.
    pub device_id_code: DeviceIdCode,
    /// Conformity level of the device.
    pub conformity_level: u8,
    /// If true, more objects follow — send another request with `next_object_id`.
    pub more_follows: bool,
    /// Next object ID to request if `more_follows` is true.
    pub next_object_id: u8,
    /// Number of object entries in this response.
    pub num_objects: u8,
    /// Raw object data bytes (packed: `[obj_id, obj_len, value...]` repeated).
    pub object_data: &'buf [u8],
}

impl<'buf> ReadDeviceIdentificationResponse<'buf> {
    /// Decode from the data bytes following the function code.
    ///
    /// # Errors
    ///
    /// Returns `DecodeError` if the data is malformed.
    pub fn decode(data: &'buf [u8]) -> Result<Self, DecodeError> {
        if data.len() < 6 {
            return Err(DecodeError::Truncated {
                expected: 6,
                actual: data.len(),
            });
        }
        if data[0] != MeiType::ReadDeviceIdentification.code() {
            return Err(DecodeError::UnknownMeiType(data[0]));
        }
        let device_id_code =
            DeviceIdCode::from_raw(data[1]).ok_or(DecodeError::InvalidDeviceIdCode(data[1]))?;
        let conformity_level = data[2];
        let more_follows = data[3] == 0xFF;
        let next_object_id = data[4];
        let num_objects = data[5];

        // Validate that object data is well-formed.
        let object_data = &data[6..];
        let mut offset = 0;
        for _ in 0..num_objects {
            if offset + 2 > object_data.len() {
                return Err(DecodeError::Truncated {
                    expected: 6 + offset + 2,
                    actual: data.len(),
                });
            }
            let obj_len = object_data[offset + 1] as usize;
            offset += 2 + obj_len;
            if offset > object_data.len() {
                return Err(DecodeError::Truncated {
                    expected: 6 + offset,
                    actual: data.len(),
                });
            }
        }

        Ok(Self {
            device_id_code,
            conformity_level,
            more_follows,
            next_object_id,
            num_objects,
            object_data,
        })
    }

    /// Iterate over the identification objects in this response.
    #[must_use]
    pub fn objects(&self) -> DeviceIdObjectIter<'buf> {
        DeviceIdObjectIter {
            data: self.object_data,
            remaining: self.num_objects,
        }
    }
}

/// Iterator over device identification object entries.
pub struct DeviceIdObjectIter<'buf> {
    data: &'buf [u8],
    remaining: u8,
}

impl<'buf> Iterator for DeviceIdObjectIter<'buf> {
    type Item = DeviceIdObjectEntry<'buf>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 || self.data.len() < 2 {
            return None;
        }
        let id = self.data[0];
        let len = self.data[1] as usize;
        if self.data.len() < 2 + len {
            self.remaining = 0;
            self.data = &[];
            return None;
        }
        let value = &self.data[2..2 + len];
        self.data = &self.data[2 + len..];
        self.remaining -= 1;
        Some(DeviceIdObjectEntry { id, value })
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    use alloc::vec::Vec;

    use super::*;

    #[test]
    fn decode_single_object_response() {
        let data = [
            0x0E, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x05, b'h', b'e', b'l', b'l', b'o',
        ];
        let resp = ReadDeviceIdentificationResponse::decode(&data).unwrap();
        assert_eq!(resp.device_id_code, DeviceIdCode::BasicStream);
        assert!(!resp.more_follows);
        assert_eq!(resp.num_objects, 1);
        let objs: Vec<_> = resp.objects().collect();
        assert_eq!(objs[0].id, 0x00);
        assert_eq!(objs[0].value, b"hello");
    }

    #[test]
    fn decode_multiple_objects() {
        let data = [
            0x0E, 0x01, 0x01, 0x00, 0x00, 0x03, 0x00, 0x04, b't', b'e', b's', b't', 0x01, 0x03,
            b'X', b'Y', b'Z', 0x02, 0x05, b'1', b'.', b'0', b'.', b'0',
        ];
        let resp = ReadDeviceIdentificationResponse::decode(&data).unwrap();
        let objs: Vec<_> = resp.objects().collect();
        assert_eq!(objs.len(), 3);
        assert_eq!(objs[0].value, b"test");
        assert_eq!(objs[1].value, b"XYZ");
        assert_eq!(objs[2].value, b"1.0.0");
    }

    #[test]
    fn decode_more_follows() {
        let data = [0x0E, 0x01, 0x01, 0xFF, 0x02, 0x01, 0x00, 0x02, b'O', b'K'];
        let resp = ReadDeviceIdentificationResponse::decode(&data).unwrap();
        assert!(resp.more_follows);
        assert_eq!(resp.next_object_id, 0x02);
    }

    #[test]
    fn decode_truncated_header() {
        let data = [0x0E, 0x01, 0x01];
        assert!(matches!(
            ReadDeviceIdentificationResponse::decode(&data),
            Err(DecodeError::Truncated { .. })
        ));
    }

    #[test]
    fn decode_truncated_object() {
        let data = [0x0E, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x05, b'h', b'i'];
        assert!(matches!(
            ReadDeviceIdentificationResponse::decode(&data),
            Err(DecodeError::Truncated { .. })
        ));
    }

    #[test]
    fn object_iterator_stops_on_malformed_manual_response() {
        let resp = ReadDeviceIdentificationResponse {
            device_id_code: DeviceIdCode::BasicStream,
            conformity_level: 0x01,
            more_follows: false,
            next_object_id: 0,
            num_objects: 1,
            object_data: &[0x00, 0x05, b'h', b'i'],
        };

        assert!(resp.objects().next().is_none());
    }
}
