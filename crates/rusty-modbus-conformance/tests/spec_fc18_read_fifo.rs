//! Spec V1.1b3 §6.18 — FC 18 (24) Read FIFO Queue conformance tests.

use rusty_modbus_codec::{decode_request, decode_response};
use rusty_modbus_codec::request::Encode;
use rusty_modbus_types::Address;

// Spec example p.40: read FIFO at pointer register 1246 (0x04DE)
// Response: 2 values (440=0x01B8, 4740=0x1284)
const SPEC_REQUEST: &[u8] = &[0x18, 0x04, 0xDE];
const SPEC_RESPONSE: &[u8] = &[0x18, 0x00, 0x06, 0x00, 0x02, 0x01, 0xB8, 0x12, 0x84];

#[test]
fn spec_6_18_request_decode() {
    match decode_request(SPEC_REQUEST).unwrap() {
        rusty_modbus_codec::RequestPdu::ReadFifoQueue(r) => {
            assert_eq!(r.fifo_pointer_address, Address(0x04DE));
        }
        other => panic!("expected ReadFifoQueue, got {other:?}"),
    }
}

#[test]
fn spec_6_18_response_decode() {
    match decode_response(SPEC_RESPONSE).unwrap() {
        rusty_modbus_codec::ResponsePdu::ReadFifoQueue(r) => {
            assert_eq!(r.byte_count, 0x0006);
            assert_eq!(r.fifo_count, 0x0002);
            // Spec: register 1247 = 440 (0x01B8), register 1248 = 4740 (0x1284)
            assert_eq!(r.fifo_values, &[0x01, 0xB8, 0x12, 0x84]);
        }
        other => panic!("expected ReadFifoQueue response, got {other:?}"),
    }
}

#[test]
fn encode_round_trip() {
    let req = rusty_modbus_codec::request::ReadFifoQueueRequest {
        fifo_pointer_address: Address(0x04DE),
    };
    let mut buf = [0u8; 3];
    let len = req.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], SPEC_REQUEST);
}

#[test]
fn truncated() {
    assert!(matches!(
        decode_request(&[0x18]),
        Err(rusty_modbus_codec::DecodeError::Truncated { .. })
    ));
}
