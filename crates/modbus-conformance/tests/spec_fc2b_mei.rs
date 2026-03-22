//! Spec V1.1b3 §6.19–6.21 — FC 2B (43) Encapsulated Interface Transport
//! and Read Device Identification conformance tests.

use modbus_codec::{decode_request, decode_response};

// ── FC 2B generic MEI ──────────────────────────────────────────────

#[test]
fn spec_6_19_mei_request_decode() {
    // Request: FC=0x2B, MEI_type=0x0E, data=[0x01, 0x00]
    let pdu = [0x2B, 0x0E, 0x01, 0x00];
    match decode_request(&pdu).unwrap() {
        modbus_codec::RequestPdu::EncapsulatedInterface(r) => {
            assert_eq!(r.mei_type, modbus_types::MeiType::ReadDeviceIdentification);
            assert_eq!(r.data, &[0x01, 0x00]);
        }
        other => panic!("expected EncapsulatedInterface, got {other:?}"),
    }
}

#[test]
fn spec_6_19_mei_response_decode() {
    // Minimal response: FC=0x2B, MEI_type=0x0E, data follows
    let pdu = [0x2B, 0x0E, 0x01, 0x00, 0x00];
    match decode_response(&pdu).unwrap() {
        modbus_codec::ResponsePdu::EncapsulatedInterface(r) => {
            assert_eq!(r.mei_type, modbus_types::MeiType::ReadDeviceIdentification);
        }
        other => panic!("expected EncapsulatedInterface response, got {other:?}"),
    }
}

// ── Read Device Identification typed codec ─────────────────────────

#[test]
fn spec_6_21_device_id_request_encode() {
    // Spec p.43-45: request FC=0x2B, MEI=0x0E, ReadDevId=0x01, ObjectId=0x00
    use modbus_codec::request::{Encode, ReadDeviceIdentificationRequest};
    use modbus_types::DeviceIdCode;

    let req = ReadDeviceIdentificationRequest {
        device_id_code: DeviceIdCode::BasicStream,
        object_id: 0x00,
    };
    let mut buf = [0u8; 4];
    let len = req.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], &[0x2B, 0x0E, 0x01, 0x00]);
}

#[test]
fn spec_6_21_device_id_response_decode() {
    // Spec example p.45: basic device ID, all in one response
    // 3 objects: VendorName(22 bytes), ProductCode(13 bytes), Revision(5 bytes)
    use modbus_codec::response::device_id::ReadDeviceIdentificationResponse;

    // After FC+MEI bytes stripped, the codec expects data starting at MEI type
    let data: &[u8] = &[
        0x0E, // MEI type
        0x01, // ReadDevId code (BasicStream)
        0x01, // Conformity level (basic)
        0x00, // More follows = NO
        0x00, // Next object id
        0x03, // Number of objects
        // Object 0: VendorName
        0x00, 0x05, b'T', b'e', b's', b't', b'!',
        // Object 1: ProductCode
        0x01, 0x03, b'X', b'Y', b'Z',
        // Object 2: Revision
        0x02, 0x05, b'1', b'.', b'0', b'.', b'0',
    ];
    let resp = ReadDeviceIdentificationResponse::decode(data).unwrap();
    assert_eq!(resp.device_id_code, modbus_types::DeviceIdCode::BasicStream);
    assert_eq!(resp.conformity_level, 0x01);
    assert!(!resp.more_follows);
    assert_eq!(resp.num_objects, 3);

    let objects: Vec<_> = resp.objects().collect();
    assert_eq!(objects[0].id, 0x00); // VendorName
    assert_eq!(objects[0].value, b"Test!");
    assert_eq!(objects[1].id, 0x01); // ProductCode
    assert_eq!(objects[1].value, b"XYZ");
    assert_eq!(objects[2].id, 0x02); // Revision
    assert_eq!(objects[2].value, b"1.0.0");
}

#[test]
fn spec_6_21_more_follows() {
    use modbus_codec::response::device_id::ReadDeviceIdentificationResponse;

    let data: &[u8] = &[
        0x0E, 0x01, 0x01,
        0xFF, // More follows = YES
        0x02, // Next object id = 2
        0x01, // 1 object in this response
        0x00, 0x02, b'O', b'K',
    ];
    let resp = ReadDeviceIdentificationResponse::decode(data).unwrap();
    assert!(resp.more_follows);
    assert_eq!(resp.next_object_id, 0x02);
}

#[test]
fn truncated() {
    assert!(matches!(
        decode_request(&[0x2B]),
        Err(modbus_codec::DecodeError::Truncated { .. })
    ));
}
