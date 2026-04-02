//! TCP Guide §3.1.3 — MBAP framing conformance tests.
//!
//! Verifies the MBAP codec against the Modbus Messaging on TCP/IP
//! Implementation Guide V1.0b §3.1.3 MBAP Header description.

use bytes::{Bytes, BytesMut};
use rusty_modbus_frame::frame::FrameHeader;
use rusty_modbus_frame::mbap::MbapCodec;
use rusty_modbus_types::{MAX_PDU_SIZE, MBAP_HEADER_LEN, MbapHeader};
use tokio_util::codec::{Decoder, Encoder};
use zerocopy::IntoBytes;

/// Build an MBAP ADU from raw header bytes + PDU for testing.
fn build_adu(txn_id: u16, unit_id: u8, pdu: &[u8]) -> Vec<u8> {
    let header = MbapHeader::new(txn_id, unit_id, u16::try_from(pdu.len()).unwrap());
    let mut buf = Vec::with_capacity(MBAP_HEADER_LEN + pdu.len());
    buf.extend_from_slice(header.as_bytes());
    buf.extend_from_slice(pdu);
    buf
}

// ── §3.1.3 MBAP Header field layout ───────────────────────────────

#[test]
fn spec_3_1_3_header_is_7_bytes() {
    // "The header is 7 bytes long"
    assert_eq!(MBAP_HEADER_LEN, 7);
}

#[test]
fn spec_3_1_3_fields_big_endian() {
    // "Remark: the different fields are encoded in Big-Endian."
    let header = MbapHeader::new(0x0102, 0xFF, 5);
    let bytes = header.as_bytes();
    // Transaction ID BE
    assert_eq!(bytes[0], 0x01);
    assert_eq!(bytes[1], 0x02);
    // Protocol ID = 0x0000 BE
    assert_eq!(bytes[2], 0x00);
    assert_eq!(bytes[3], 0x00);
    // Length = pdu_len(5) + 1 = 6 BE
    assert_eq!(bytes[4], 0x00);
    assert_eq!(bytes[5], 0x06);
    // Unit ID
    assert_eq!(bytes[6], 0xFF);
}

#[test]
fn spec_3_1_3_protocol_id_zero() {
    // "Protocol Identifier: 0 = MODBUS protocol"
    let header = MbapHeader::new(1, 1, 5);
    assert_eq!(header.protocol_id.get(), 0x0000);
}

#[test]
fn spec_3_1_3_length_includes_unit_id() {
    // "Length: Number of following bytes" — includes unit_id + PDU
    let header = MbapHeader::new(1, 1, 5); // pdu_len = 5
    // length field = pdu_len + 1 (for unit_id) = 6
    assert_eq!(header.length.get(), 6);
    assert_eq!(header.pdu_length(), 5);
}

#[test]
fn spec_3_1_3_transaction_id_echoed() {
    // "Transaction Identifier: the MODBUS server copies in the response
    //  the transaction identifier of the request"
    let txn_id = 0x1234;
    let adu = build_adu(txn_id, 0xFF, &[0x03, 0x00, 0x6B, 0x00, 0x03]);
    let mut buf = BytesMut::from(&adu[..]);
    let mut codec = MbapCodec;
    let frame = codec.decode(&mut buf).unwrap().unwrap();
    match frame.header {
        FrameHeader::Mbap(h) => assert_eq!(h.transaction_id.get(), txn_id),
        _ => panic!("expected MBAP header"),
    }
}

// ── MBAP codec decode conformance ──────────────────────────────────

#[test]
fn spec_3_1_3_decode_fc03_example() {
    // FC03 Read Holding Registers request wrapped in MBAP
    // TxnId=1, UnitId=0xFF, PDU=[0x03, 0x00, 0x6B, 0x00, 0x03]
    let adu = build_adu(1, 0xFF, &[0x03, 0x00, 0x6B, 0x00, 0x03]);
    assert_eq!(adu.len(), 12); // 7 header + 5 PDU

    let mut buf = BytesMut::from(&adu[..]);
    let mut codec = MbapCodec;
    let frame = codec.decode(&mut buf).unwrap().unwrap();

    assert_eq!(frame.unit_id(), 0xFF);
    assert_eq!(&frame.pdu[..], &[0x03, 0x00, 0x6B, 0x00, 0x03]);
    assert!(buf.is_empty());
}

#[test]
fn spec_3_1_3_decode_fc03_response() {
    // FC03 response: regs 108-110 = [555, 0, 100]
    let pdu = [0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64];
    let adu = build_adu(1, 0xFF, &pdu);
    assert_eq!(adu.len(), 15); // 7 + 8

    let mut buf = BytesMut::from(&adu[..]);
    let mut codec = MbapCodec;
    let frame = codec.decode(&mut buf).unwrap().unwrap();
    assert_eq!(&frame.pdu[..], &pdu);
}

#[test]
fn spec_adu_total_calculation() {
    // Total ADU = 6 + length_field (per §3.1.3: 6 bytes before length payload)
    // length_field = unit_id(1) + PDU bytes
    let pdu = [0x01, 0x00, 0x13, 0x00, 0x13]; // FC01 5 bytes
    let adu = build_adu(42, 1, &pdu);
    // length_field = 1 + 5 = 6
    // total = 6 + 6 = 12
    assert_eq!(adu.len(), 12);
    assert_eq!(adu.len(), 6 + 6); // explicit formula
}

#[test]
fn invalid_protocol_id_rejected() {
    // Protocol ID must be 0x0000
    let mut adu = build_adu(1, 1, &[0x03]);
    adu[2] = 0x00;
    adu[3] = 0x01; // proto_id = 1

    let mut buf = BytesMut::from(&adu[..]);
    let mut codec = MbapCodec;
    assert!(matches!(
        codec.decode(&mut buf),
        Err(rusty_modbus_frame::FrameError::InvalidProtocolId(1))
    ));
}

#[test]
fn length_overflow_rejected() {
    // Max length field = MAX_PDU_SIZE + 1 = 254
    let mut adu = build_adu(1, 1, &[0x03]);
    let overflow: u16 = u16::try_from(MAX_PDU_SIZE).unwrap() + 2; // 255
    adu[4] = (overflow >> 8) as u8;
    adu[5] = (overflow & 0xFF) as u8;

    let mut buf = BytesMut::from(&adu[..]);
    let mut codec = MbapCodec;
    assert!(matches!(
        codec.decode(&mut buf),
        Err(rusty_modbus_frame::FrameError::LengthOverflow(_))
    ));
}

#[test]
fn partial_header_waits() {
    let mut buf = BytesMut::from(&[0x00, 0x01, 0x00, 0x00][..]);
    let mut codec = MbapCodec;
    assert!(codec.decode(&mut buf).unwrap().is_none());
}

#[test]
fn partial_body_waits() {
    let adu = build_adu(1, 1, &[0x03, 0x00, 0x00, 0x00, 0x0A]);
    let mut buf = BytesMut::from(&adu[..adu.len() - 2]); // chop 2 bytes
    let mut codec = MbapCodec;
    assert!(codec.decode(&mut buf).unwrap().is_none());
}

#[test]
fn multiple_frames_split_correctly() {
    let adu1 = build_adu(1, 1, &[0x03, 0x02, 0x00, 0x64]);
    let adu2 = build_adu(2, 2, &[0x06, 0x00, 0x01, 0x00, 0x03]);
    let mut buf = BytesMut::new();
    buf.extend_from_slice(&adu1);
    buf.extend_from_slice(&adu2);

    let mut codec = MbapCodec;
    let f1 = codec.decode(&mut buf).unwrap().unwrap();
    let f2 = codec.decode(&mut buf).unwrap().unwrap();
    assert_eq!(f1.unit_id(), 1);
    assert_eq!(f2.unit_id(), 2);
    assert!(buf.is_empty());
}

// ── Encode round-trip ──────────────────────────────────────────────

#[test]
fn encode_decode_round_trip() {
    let pdu = Bytes::from_static(&[0x01, 0x00, 0x13, 0x00, 0x13]);
    let header = MbapHeader::new(100, 0xFF, u16::try_from(pdu.len()).unwrap());
    let frame = rusty_modbus_frame::Frame {
        header: FrameHeader::Mbap(header),
        pdu: pdu.clone(),
    };

    let mut buf = BytesMut::new();
    let mut codec = MbapCodec;
    codec.encode(frame, &mut buf).unwrap();

    let decoded = codec.decode(&mut buf).unwrap().unwrap();
    match decoded.header {
        FrameHeader::Mbap(h) => {
            assert_eq!(h.transaction_id.get(), 100);
            assert_eq!(h.protocol_id.get(), 0);
            assert_eq!(h.unit_id, 0xFF);
        }
        _ => panic!("expected MBAP"),
    }
    assert_eq!(&decoded.pdu[..], &pdu[..]);
}

#[test]
fn zero_length_pdu() {
    // length=1 means only unit_id, zero PDU bytes
    let adu = build_adu(1, 0x01, &[]);
    let mut buf = BytesMut::from(&adu[..]);
    let mut codec = MbapCodec;
    let frame = codec.decode(&mut buf).unwrap().unwrap();
    assert!(frame.pdu.is_empty());
}
