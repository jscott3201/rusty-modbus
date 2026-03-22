//! RTU-over-TCP codec for Modbus framing.
//!
//! Carries RTU frames inside a TCP stream. Since TCP has no inter-character
//! silence for frame boundary detection, this codec uses a CRC-scanning
//! approach: it buffers incoming data and tries progressively longer candidate
//! frames (starting at 4 bytes) until a valid CRC-16 is found. This is how
//! many production RTU-over-TCP implementations work.

use bytes::{BufMut, BytesMut};
use rusty_modbus_types::MAX_RTU_ADU_SIZE;
use tokio_util::codec::{Decoder, Encoder};

use crate::crc::{crc16, verify_crc};
use crate::error::FrameError;
use crate::frame::{Frame, FrameHeader};

/// Minimum RTU frame size: `unit_id`(1) + FC(1) + CRC(2).
const MIN_RTU_FRAME: usize = 4;

/// RTU-over-TCP codec.
///
/// Uses CRC-based length detection since TCP has no inter-character silence
/// for frame boundaries. For each decode attempt, candidate frame lengths from
/// 4 up to `MAX_RTU_ADU_SIZE` (or buffer length, whichever is smaller) are
/// tested until a valid CRC-16 is found.
#[derive(Debug, Default)]
pub struct RtuOverTcpCodec;

impl Decoder for RtuOverTcpCodec {
    type Item = Frame;
    type Error = FrameError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < MIN_RTU_FRAME {
            return Ok(None);
        }

        let max_len = src.len().min(MAX_RTU_ADU_SIZE);

        // Scan candidate frame lengths from smallest to largest.
        // The first length whose trailing 2 bytes form a valid CRC-16 of the
        // preceding data is accepted as the frame boundary.
        for candidate_len in MIN_RTU_FRAME..=max_len {
            let candidate = &src[..candidate_len];

            if verify_crc(candidate) {
                let unit_id = src[0];

                let adu = src.split_to(candidate_len).freeze();
                let pdu = adu.slice(1..adu.len() - 2);

                return Ok(Some(Frame {
                    header: FrameHeader::Rtu { unit_id },
                    pdu,
                }));
            }
        }

        // If the buffer has grown beyond MAX_RTU_ADU_SIZE without a valid CRC
        // match, the data is corrupt or mis-framed — discard what we cannot use.
        if src.len() > MAX_RTU_ADU_SIZE {
            return Err(FrameError::Truncated);
        }

        // Not enough data yet — ask for more.
        Ok(None)
    }
}

impl Encoder<Frame> for RtuOverTcpCodec {
    type Error = FrameError;

    fn encode(&mut self, item: Frame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let unit_id = match item.header {
            FrameHeader::Rtu { unit_id } => unit_id,
            FrameHeader::Mbap(h) => h.unit_id,
        };

        // Reserve space: unit_id(1) + PDU + CRC(2).
        dst.reserve(1 + item.pdu.len() + 2);

        dst.put_u8(unit_id);
        dst.put_slice(&item.pdu);

        // CRC-16 over [unit_id, pdu...].
        let crc_start = dst.len() - 1 - item.pdu.len();
        let crc = crc16(&dst[crc_start..]);
        dst.put_u16_le(crc);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a valid RTU frame: [`unit_id`, pdu..., `crc_lo`, `crc_hi`].
    fn make_rtu_frame(unit_id: u8, pdu: &[u8]) -> Vec<u8> {
        let mut buf = vec![unit_id];
        buf.extend_from_slice(pdu);
        let crc = crc16(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());
        buf
    }

    #[test]
    fn decode_single_frame() {
        let raw = make_rtu_frame(0x01, &[0x03, 0x00, 0x00, 0x00, 0x0A]);
        let mut buf = BytesMut::from(&raw[..]);
        let mut codec = RtuOverTcpCodec;

        let frame = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.unit_id(), 0x01);
        assert_eq!(&frame.pdu[..], &[0x03, 0x00, 0x00, 0x00, 0x0A]);
        assert!(buf.is_empty());
    }

    #[test]
    fn decode_two_back_to_back_frames() {
        let frame1 = make_rtu_frame(0x01, &[0x03, 0x02, 0x00, 0x64]);
        let frame2 = make_rtu_frame(0x02, &[0x06, 0x00, 0x01, 0x00, 0x03]);

        let mut buf = BytesMut::new();
        buf.extend_from_slice(&frame1);
        buf.extend_from_slice(&frame2);

        let mut codec = RtuOverTcpCodec;

        let f1 = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(f1.unit_id(), 0x01);

        let f2 = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(f2.unit_id(), 0x02);

        assert!(buf.is_empty());
    }

    #[test]
    fn decode_incomplete_returns_none() {
        let raw = make_rtu_frame(0x01, &[0x03, 0x00]);
        // Feed only the first 3 bytes (incomplete).
        let mut buf = BytesMut::from(&raw[..3]);
        let mut codec = RtuOverTcpCodec;

        assert!(codec.decode(&mut buf).unwrap().is_none());
    }

    #[test]
    fn decode_partial_then_complete() {
        let raw = make_rtu_frame(0x01, &[0x03, 0x02, 0xAB, 0xCD]);
        let mut buf = BytesMut::new();
        let mut codec = RtuOverTcpCodec;

        // Feed partial data.
        buf.extend_from_slice(&raw[..4]);
        assert!(codec.decode(&mut buf).unwrap().is_none());

        // Feed the rest.
        buf.extend_from_slice(&raw[4..]);
        let frame = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.unit_id(), 0x01);
        assert_eq!(&frame.pdu[..], &[0x03, 0x02, 0xAB, 0xCD]);
    }

    #[test]
    fn encode_roundtrip() {
        let original_pdu = vec![0x03, 0x02, 0x00, 0x64];
        let frame = Frame {
            header: FrameHeader::Rtu { unit_id: 0x01 },
            pdu: bytes::Bytes::from(original_pdu.clone()),
        };

        let mut dst = BytesMut::new();
        let mut codec = RtuOverTcpCodec;
        codec.encode(frame, &mut dst).unwrap();

        // Decode the encoded frame.
        let decoded = codec.decode(&mut dst).unwrap().unwrap();
        assert_eq!(decoded.unit_id(), 0x01);
        assert_eq!(&decoded.pdu[..], &original_pdu[..]);
    }

    #[test]
    fn decode_exception_response() {
        // Exception: unit_id=0x01, FC=0x83 (0x03|0x80), exception_code=0x02
        let raw = make_rtu_frame(0x01, &[0x83, 0x02]);
        let mut buf = BytesMut::from(&raw[..]);
        let mut codec = RtuOverTcpCodec;

        let frame = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.unit_id(), 0x01);
        assert_eq!(&frame.pdu[..], &[0x83, 0x02]);
    }

    #[test]
    fn overflow_returns_error() {
        // Fill buffer with random-looking data beyond MAX_RTU_ADU_SIZE that
        // won't accidentally form a valid CRC.
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&vec![0xAA; MAX_RTU_ADU_SIZE + 1]);
        let mut codec = RtuOverTcpCodec;

        let err = codec.decode(&mut buf).unwrap_err();
        assert!(matches!(err, FrameError::Truncated));
    }
}
