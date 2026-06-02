//! RTU framing conformance tests.
//!
//! Verifies RTU and RTU-over-TCP codecs against the Modbus serial line
//! framing specification: unit_id(1) + PDU + CRC-16/LE(2).

use bytes::{Bytes, BytesMut};
use rusty_modbus_frame::Frame;
use rusty_modbus_frame::crc::crc16;
use rusty_modbus_frame::frame::FrameHeader;
use rusty_modbus_types::MAX_PDU_SIZE;
use tokio_util::codec::{Decoder, Encoder};

/// Build a valid RTU frame.
fn make_rtu_frame(unit_id: u8, pdu: &[u8]) -> Vec<u8> {
    let mut buf = vec![unit_id];
    buf.extend_from_slice(pdu);
    let crc = crc16(&buf);
    buf.extend_from_slice(&crc.to_le_bytes());
    buf
}

// ── RTU Codec ──────────────────────────────────────────────────────

#[test]
fn rtu_decode_fc03_request() {
    // Slave 1, FC03, start 0x006B, qty 3 — spec §6.3 example
    let raw = make_rtu_frame(0x01, &[0x03, 0x00, 0x6B, 0x00, 0x03]);
    let mut buf = BytesMut::from(&raw[..]);
    let mut codec = rusty_modbus_frame::rtu::RtuCodec;

    let frame = codec.decode(&mut buf).unwrap().unwrap();
    assert_eq!(frame.unit_id(), 0x01);
    assert_eq!(&frame.pdu[..], &[0x03, 0x00, 0x6B, 0x00, 0x03]);
    assert!(buf.is_empty());
}

#[test]
fn rtu_crc_little_endian_on_wire() {
    // CRC for [0x01, 0x03, 0x00, 0x00, 0x00, 0x0A] = 0xCDC5
    // On wire: [0xC5, 0xCD] (little-endian)
    let raw = make_rtu_frame(0x01, &[0x03, 0x00, 0x00, 0x00, 0x0A]);
    let crc_offset = raw.len() - 2;
    assert_eq!(raw[crc_offset], 0xC5); // low byte
    assert_eq!(raw[crc_offset + 1], 0xCD); // high byte
}

#[test]
fn rtu_min_frame_size() {
    // Minimum: unit_id(1) + FC(1) + CRC(2) = 4 bytes
    let mut buf = BytesMut::from(&[0x01, 0x07, 0x00][..]); // 3 bytes
    let mut codec = rusty_modbus_frame::rtu::RtuCodec;
    assert!(codec.decode(&mut buf).unwrap().is_none());
}

#[test]
fn rtu_bad_crc_error() {
    let mut raw = make_rtu_frame(0x01, &[0x03]);
    let last = raw.len() - 1;
    raw[last] ^= 0xFF; // corrupt CRC
    let mut buf = BytesMut::from(&raw[..]);
    let mut codec = rusty_modbus_frame::rtu::RtuCodec;
    assert!(matches!(
        codec.decode(&mut buf),
        Err(rusty_modbus_frame::FrameError::CrcMismatch { .. })
    ));
}

#[test]
fn rtu_rejects_adu_over_256_bytes() {
    let raw = make_rtu_frame(0x01, &vec![0x03; MAX_PDU_SIZE + 1]);
    let mut buf = BytesMut::from(&raw[..]);
    let mut codec = rusty_modbus_frame::rtu::RtuCodec;

    assert!(matches!(
        codec.decode(&mut buf),
        Err(rusty_modbus_frame::FrameError::PduLengthOverflow { .. })
    ));
}

#[test]
fn rtu_encode_decode_round_trip() {
    let pdu = Bytes::from_static(&[0x05, 0x00, 0xAC, 0xFF, 0x00]);
    let frame = Frame {
        header: FrameHeader::Rtu { unit_id: 0x01 },
        pdu: pdu.clone(),
    };

    let mut dst = BytesMut::new();
    let mut codec = rusty_modbus_frame::rtu::RtuCodec;
    codec.encode(frame, &mut dst).unwrap();

    let decoded = codec.decode(&mut dst).unwrap().unwrap();
    assert_eq!(decoded.unit_id(), 0x01);
    assert_eq!(&decoded.pdu[..], &pdu[..]);
}

// ── RTU-over-TCP Codec ────────────────────────────────────────────

#[test]
fn rtu_tcp_decode_with_crc_scanning() {
    let raw = make_rtu_frame(0x01, &[0x03, 0x02, 0x00, 0x64]);
    let mut buf = BytesMut::from(&raw[..]);
    let mut codec = rusty_modbus_frame::rtu_tcp::RtuOverTcpCodec;

    let frame = codec.decode(&mut buf).unwrap().unwrap();
    assert_eq!(frame.unit_id(), 0x01);
    assert_eq!(&frame.pdu[..], &[0x03, 0x02, 0x00, 0x64]);
}

#[test]
fn rtu_tcp_back_to_back_frames() {
    let f1 = make_rtu_frame(0x01, &[0x03, 0x02, 0x00, 0x64]);
    let f2 = make_rtu_frame(0x02, &[0x06, 0x00, 0x01, 0x00, 0x03]);

    let mut buf = BytesMut::new();
    buf.extend_from_slice(&f1);
    buf.extend_from_slice(&f2);

    let mut codec = rusty_modbus_frame::rtu_tcp::RtuOverTcpCodec;

    let frame1 = codec.decode(&mut buf).unwrap().unwrap();
    assert_eq!(frame1.unit_id(), 0x01);
    let frame2 = codec.decode(&mut buf).unwrap().unwrap();
    assert_eq!(frame2.unit_id(), 0x02);
    assert!(buf.is_empty());
}

#[test]
fn rtu_tcp_incremental_delivery() {
    let raw = make_rtu_frame(0x01, &[0x03, 0x02, 0xAB, 0xCD]);
    let mut buf = BytesMut::new();
    let mut codec = rusty_modbus_frame::rtu_tcp::RtuOverTcpCodec;

    // Feed 4 bytes — not enough for a valid CRC match
    buf.extend_from_slice(&raw[..4]);
    assert!(codec.decode(&mut buf).unwrap().is_none());

    // Feed the rest
    buf.extend_from_slice(&raw[4..]);
    let frame = codec.decode(&mut buf).unwrap().unwrap();
    assert_eq!(frame.unit_id(), 0x01);
}

#[test]
fn rtu_tcp_overflow_detection() {
    let mut buf = BytesMut::from(&vec![0xAA; 257][..]);
    let mut codec = rusty_modbus_frame::rtu_tcp::RtuOverTcpCodec;
    assert!(matches!(
        codec.decode(&mut buf),
        Err(rusty_modbus_frame::FrameError::Truncated)
    ));
}

#[test]
fn rtu_tcp_exception_response() {
    // Unit 1, FC83 (exception), code 0x02
    let raw = make_rtu_frame(0x01, &[0x83, 0x02]);
    let mut buf = BytesMut::from(&raw[..]);
    let mut codec = rusty_modbus_frame::rtu_tcp::RtuOverTcpCodec;

    let frame = codec.decode(&mut buf).unwrap().unwrap();
    assert_eq!(&frame.pdu[..], &[0x83, 0x02]);
}

#[test]
fn rtu_encode_from_mbap_header_uses_unit_id() {
    // RTU encoder should extract unit_id from MBAP header if given one
    use rusty_modbus_types::MbapHeader;
    let pdu = Bytes::from_static(&[0x03, 0x00, 0x00, 0x00, 0x01]);
    let frame = Frame {
        header: FrameHeader::Mbap(MbapHeader::new(1, 0x05, u16::try_from(pdu.len()).unwrap())),
        pdu: pdu.clone(),
    };

    let mut dst = BytesMut::new();
    let mut codec = rusty_modbus_frame::rtu::RtuCodec;
    codec.encode(frame, &mut dst).unwrap();

    let decoded = codec.decode(&mut dst).unwrap().unwrap();
    assert_eq!(decoded.unit_id(), 0x05);
    assert_eq!(&decoded.pdu[..], &pdu[..]);
}
