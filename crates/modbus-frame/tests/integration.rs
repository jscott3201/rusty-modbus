//! Integration checkpoint: raw TCP bytes → MbapCodec → Frame → codec decode → typed data.
//!
//! Proves the three-layer pipeline (modbus-types → modbus-codec → modbus-frame) composes.

use bytes::BytesMut;
use modbus_codec::{decode_response, ResponsePdu};
use modbus_frame::{MbapCodec, OwnedResponsePdu};
use modbus_types::MbapHeader;
use tokio_util::codec::Decoder;
use zerocopy::IntoBytes;

/// Build a complete MBAP ADU from a PDU byte slice.
fn build_mbap_adu(transaction_id: u16, unit_id: u8, pdu: &[u8]) -> Vec<u8> {
    let header = MbapHeader::new(transaction_id, unit_id, pdu.len() as u16);
    let mut buf = Vec::with_capacity(7 + pdu.len());
    buf.extend_from_slice(header.as_bytes());
    buf.extend_from_slice(pdu);
    buf
}

#[test]
fn end_to_end_tcp_read_holding_registers_response() {
    // Step 1: Build a raw TCP byte stream containing a ReadHoldingRegisters response.
    // PDU: FC 0x03, byte_count=6, registers: [0x022B, 0x0000, 0x0064]
    let pdu = [0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64];
    let adu = build_mbap_adu(0x0001, 0xFF, &pdu);

    // Step 2: Feed into MbapCodec decoder.
    let mut codec = MbapCodec;
    let mut src = BytesMut::from(&adu[..]);
    let frame = codec.decode(&mut src).unwrap().expect("should decode a frame");

    // Verify frame header.
    assert_eq!(frame.unit_id(), 0xFF);
    assert_eq!(frame.pdu.len(), 8);

    // Step 3: Decode the PDU through the codec layer (borrowed types).
    let response = decode_response(&frame.pdu).unwrap();
    match response {
        ResponsePdu::ReadHoldingRegisters(rhr) => {
            assert_eq!(rhr.count(), 3);
            assert_eq!(rhr.register(0), 0x022B);
            assert_eq!(rhr.register(1), 0x0000);
            assert_eq!(rhr.register(2), 0x0064);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
fn end_to_end_owned_response_conversion() {
    // Same PDU as above.
    let pdu = [0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64];
    let adu = build_mbap_adu(0x0001, 0xFF, &pdu);

    let mut codec = MbapCodec;
    let mut src = BytesMut::from(&adu[..]);
    let frame = codec.decode(&mut src).unwrap().unwrap();

    // Convert to owned response (Bytes-backed, 'static).
    let owned = OwnedResponsePdu::from_pdu(frame.pdu).unwrap();
    match owned {
        OwnedResponsePdu::ReadHoldingRegisters(rhr) => {
            assert_eq!(rhr.count(), 3);
            assert_eq!(rhr.register(0), 0x022B);
            assert_eq!(rhr.register(1), 0x0000);
            assert_eq!(rhr.register(2), 0x0064);
        }
        other => panic!("unexpected owned response: {other:?}"),
    }
}

#[test]
fn end_to_end_exception_response() {
    // Exception: FC 0x83 (ReadHoldingRegisters exception), code 0x02 (IllegalDataAddress).
    let pdu = [0x83, 0x02];
    let adu = build_mbap_adu(0x0042, 0x01, &pdu);

    let mut codec = MbapCodec;
    let mut src = BytesMut::from(&adu[..]);
    let frame = codec.decode(&mut src).unwrap().unwrap();

    // Via borrowed codec.
    let response = decode_response(&frame.pdu).unwrap();
    match response {
        ResponsePdu::Exception(exc) => {
            assert_eq!(exc.function_code, modbus_types::FunctionCode::ReadHoldingRegisters);
            assert_eq!(exc.exception_code, modbus_types::ExceptionCode::IllegalDataAddress);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn end_to_end_partial_reads() {
    // Feed the MBAP frame byte-by-byte to prove the codec handles partial reads.
    let pdu = [0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64];
    let adu = build_mbap_adu(0x0001, 0xFF, &pdu);

    let mut codec = MbapCodec;
    let mut src = BytesMut::new();

    // Feed one byte at a time — codec should return None until complete.
    for (i, &byte) in adu.iter().enumerate() {
        src.extend_from_slice(&[byte]);
        let result = codec.decode(&mut src).unwrap();
        if i < adu.len() - 1 {
            assert!(result.is_none(), "should be None at byte {i}");
        } else {
            let frame = result.expect("should decode on final byte");
            assert_eq!(frame.pdu.len(), 8);
        }
    }
}

#[test]
fn end_to_end_multiple_frames_in_stream() {
    // Two complete frames concatenated.
    let pdu1 = [0x03, 0x02, 0x00, 0x0A]; // ReadHoldingRegisters, 1 register = 0x000A
    let pdu2 = [0x01, 0x01, 0x01]; // ReadCoils, byte_count=1, status=0x01

    let mut stream = Vec::new();
    stream.extend_from_slice(&build_mbap_adu(1, 0xFF, &pdu1));
    stream.extend_from_slice(&build_mbap_adu(2, 0x01, &pdu2));

    let mut codec = MbapCodec;
    let mut src = BytesMut::from(&stream[..]);

    let f1 = codec.decode(&mut src).unwrap().unwrap();
    let f2 = codec.decode(&mut src).unwrap().unwrap();

    assert_eq!(f1.unit_id(), 0xFF);
    assert_eq!(f2.unit_id(), 0x01);

    // Verify typed decode of both.
    match decode_response(&f1.pdu).unwrap() {
        ResponsePdu::ReadHoldingRegisters(r) => assert_eq!(r.register(0), 0x000A),
        other => panic!("unexpected: {other:?}"),
    }
    match decode_response(&f2.pdu).unwrap() {
        ResponsePdu::ReadCoils(r) => assert!(r.coil(0)),
        other => panic!("unexpected: {other:?}"),
    }
}
