//! RTU serial codec for Modbus framing.
//!
//! Frame boundaries on serial are determined by inter-character silence
//! (3.5 character times), which is handled by the transport layer.
//! This codec validates the CRC-16 and extracts the PDU from a complete frame.

use bytes::{BufMut, BytesMut};
use rusty_modbus_types::{MAX_PDU_SIZE, MAX_RTU_ADU_SIZE};
use tokio_util::codec::{Decoder, Encoder};

use crate::crc::{crc16, verify_crc};
use crate::error::FrameError;
use crate::frame::{Frame, FrameHeader};

/// Minimum RTU frame size: `unit_id`(1) + FC(1) + CRC(2).
const MIN_RTU_FRAME: usize = 4;
const MIN_PDU_LENGTH: usize = 1;

/// RTU codec for serial Modbus framing.
///
/// Frame boundaries are determined by inter-character silence (handled by the
/// transport layer). This codec validates CRC and extracts the PDU.
#[derive(Debug, Default)]
pub struct RtuCodec;

impl Decoder for RtuCodec {
    type Item = Frame;
    type Error = FrameError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // Need at least unit_id + FC + CRC_lo + CRC_hi.
        if src.len() < MIN_RTU_FRAME {
            return Ok(None);
        }
        if src.len() > MAX_RTU_ADU_SIZE {
            return Err(FrameError::PduLengthOverflow {
                length: src.len() - 3,
                maximum: MAX_PDU_SIZE,
            });
        }

        // For serial transport the entire frame is in the buffer (the transport
        // delivers complete frames after silence detection). Validate CRC over
        // the whole buffer.
        if !verify_crc(src) {
            let data_end = src.len() - 2;
            let expected = crc16(&src[..data_end]);
            let actual = u16::from_le_bytes([src[data_end], src[data_end + 1]]);
            return Err(FrameError::CrcMismatch { expected, actual });
        }

        let unit_id = src[0];

        // Consume the entire buffer and freeze into a Bytes handle.
        let adu = src.split_to(src.len()).freeze();

        // PDU is everything between the unit_id byte and the trailing 2-byte CRC.
        let pdu = adu.slice(1..adu.len() - 2);

        Ok(Some(Frame {
            header: FrameHeader::Rtu { unit_id },
            pdu,
        }))
    }
}

impl Encoder<Frame> for RtuCodec {
    type Error = FrameError;

    fn encode(&mut self, item: Frame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let unit_id = match item.header {
            FrameHeader::Rtu { unit_id } => unit_id,
            FrameHeader::Mbap(h) => h.unit_id,
        };
        validate_outgoing_pdu(item.pdu.len())?;

        // Reserve space: unit_id(1) + PDU + CRC(2).
        dst.reserve(1 + item.pdu.len() + 2);

        dst.put_u8(unit_id);
        dst.put_slice(&item.pdu);

        // CRC-16 is computed over [unit_id, pdu...].
        // Build a temporary slice from what we just wrote.
        let crc_start = dst.len() - 1 - item.pdu.len();
        let crc = crc16(&dst[crc_start..]);
        dst.put_u16_le(crc);

        Ok(())
    }
}

fn validate_outgoing_pdu(pdu_len: usize) -> Result<(), FrameError> {
    if pdu_len < MIN_PDU_LENGTH {
        return Err(FrameError::InvalidPduLength {
            length: pdu_len,
            minimum: MIN_PDU_LENGTH,
        });
    }
    if pdu_len > MAX_PDU_SIZE {
        return Err(FrameError::PduLengthOverflow {
            length: pdu_len,
            maximum: MAX_PDU_SIZE,
        });
    }
    Ok(())
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
    fn decode_valid_frame() {
        let raw = make_rtu_frame(0x01, &[0x03, 0x00, 0x00, 0x00, 0x0A]);
        let mut buf = BytesMut::from(&raw[..]);
        let mut codec = RtuCodec;

        let frame = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.unit_id(), 0x01);
        assert_eq!(&frame.pdu[..], &[0x03, 0x00, 0x00, 0x00, 0x0A]);
        assert!(buf.is_empty());
    }

    #[test]
    fn decode_too_short() {
        let mut buf = BytesMut::from(&[0x01, 0x03, 0xCD][..]);
        let mut codec = RtuCodec;

        assert!(codec.decode(&mut buf).unwrap().is_none());
    }

    #[test]
    fn decode_bad_crc() {
        let mut raw = make_rtu_frame(0x01, &[0x03, 0x00]);
        // Corrupt the CRC.
        let last = raw.len() - 1;
        raw[last] ^= 0xFF;

        let mut buf = BytesMut::from(&raw[..]);
        let mut codec = RtuCodec;

        let err = codec.decode(&mut buf).unwrap_err();
        assert!(matches!(err, FrameError::CrcMismatch { .. }));
    }

    #[test]
    fn encode_roundtrip() {
        let original_pdu = vec![0x03, 0x00, 0x00, 0x00, 0x0A];
        let frame = Frame {
            header: FrameHeader::Rtu { unit_id: 0x01 },
            pdu: bytes::Bytes::from(original_pdu.clone()),
        };

        let mut dst = BytesMut::new();
        let mut codec = RtuCodec;
        codec.encode(frame, &mut dst).unwrap();

        // Decode the encoded frame.
        let decoded = codec.decode(&mut dst).unwrap().unwrap();
        assert_eq!(decoded.unit_id(), 0x01);
        assert_eq!(&decoded.pdu[..], &original_pdu[..]);
    }

    #[test]
    fn encode_rejects_empty_pdu() {
        let frame = Frame {
            header: FrameHeader::Rtu { unit_id: 0x01 },
            pdu: bytes::Bytes::new(),
        };

        let mut dst = BytesMut::new();
        let mut codec = RtuCodec;

        let err = codec.encode(frame, &mut dst).unwrap_err();
        assert!(matches!(err, FrameError::InvalidPduLength { .. }));
    }

    #[test]
    fn encode_rejects_oversized_pdu() {
        let frame = Frame {
            header: FrameHeader::Rtu { unit_id: 0x01 },
            pdu: bytes::Bytes::from(vec![0x03; MAX_PDU_SIZE + 1]),
        };

        let mut dst = BytesMut::new();
        let mut codec = RtuCodec;

        let err = codec.encode(frame, &mut dst).unwrap_err();
        assert!(matches!(err, FrameError::PduLengthOverflow { .. }));
    }

    #[test]
    fn decode_rejects_oversized_frame_even_with_valid_crc() {
        let raw = make_rtu_frame(0x01, &vec![0x03; MAX_PDU_SIZE + 1]);
        let mut buf = BytesMut::from(&raw[..]);
        let mut codec = RtuCodec;

        let err = codec.decode(&mut buf).unwrap_err();
        assert!(matches!(err, FrameError::PduLengthOverflow { .. }));
    }
}
