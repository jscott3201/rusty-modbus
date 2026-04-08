//! MBAP codec for Modbus/TCP framing.
//!
//! Implements [`tokio_util::codec::Decoder`] and [`tokio_util::codec::Encoder`]
//! over the 7-byte MBAP header defined in the Modbus TCP/IP Implementation Guide.
//! Decoding is zero-copy: the PDU is returned as a frozen [`bytes::Bytes`] handle
//! sliced directly from the read buffer.

use bytes::{BufMut, Bytes, BytesMut};
use rusty_modbus_types::{MAX_PDU_SIZE, MBAP_HEADER_LEN, MODBUS_PROTOCOL_ID, MbapHeader};
use tokio_util::codec::{Decoder, Encoder};
use zerocopy::{FromBytes, IntoBytes};

use crate::error::FrameError;
use crate::frame::{Frame, FrameHeader};

/// Maximum value of the MBAP length field: `MAX_PDU_SIZE` (253) + 1 byte for unit ID.
#[allow(clippy::cast_possible_truncation)]
const MAX_MBAP_LENGTH: u16 = MAX_PDU_SIZE as u16 + 1;

/// MBAP codec for Modbus/TCP framing.
///
/// Handles the 7-byte MBAP header (transaction ID, protocol ID, length, unit ID)
/// followed by the PDU (function code + data). The decoder validates the protocol
/// identifier and length field before yielding a [`Frame`].
#[derive(Debug, Default)]
pub struct MbapCodec;

impl Decoder for MbapCodec {
    type Item = Frame;
    type Error = FrameError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // Step 1: need at least the MBAP header to proceed.
        if src.len() < MBAP_HEADER_LEN {
            return Ok(None);
        }

        // Step 2: peek at the header fields via zero-copy overlay.
        let header = MbapHeader::ref_from_bytes(&src[..MBAP_HEADER_LEN])
            .map_err(|_| FrameError::Truncated)?;

        // Step 3: validate protocol identifier.
        let proto = header.protocol_id.get();
        if proto != MODBUS_PROTOCOL_ID {
            return Err(FrameError::InvalidProtocolId(proto));
        }

        // Step 4: validate length field bounds.
        // The length field includes the unit ID byte, so the PDU portion is length - 1.
        // Minimum: 1 (unit ID only, zero-length PDU). A value of 0 would underflow
        // the ADU size calculation and panic on the subsequent header slice.
        // Maximum allowed: MAX_PDU_SIZE (253) bytes of PDU + 1 byte unit ID = 254.
        let length = header.length.get();
        if length < 1 {
            return Err(FrameError::Truncated);
        }
        if length > MAX_MBAP_LENGTH {
            return Err(FrameError::LengthOverflow(length));
        }

        // Step 5: compute total ADU size.
        // The length field counts bytes after itself: unit_id(1) + PDU.
        // Bytes before the length-field payload: txn_id(2) + proto_id(2) + length(2) = 6.
        let total = (MBAP_HEADER_LEN - 1) + length as usize;

        // Step 6: wait for the complete frame.
        if src.len() < total {
            src.reserve(total - src.len());
            return Ok(None);
        }

        // Step 7: split the complete ADU from the buffer — O(1), no copy.
        let adu = src.split_to(total).freeze();

        // Step 8: copy the 7-byte header (MbapHeader is Copy).
        let header =
            *MbapHeader::ref_from_bytes(&adu[..MBAP_HEADER_LEN]).expect("header already validated");

        // Step 9: slice the PDU (function code + data) — zero-copy via Bytes::slice.
        let pdu = adu.slice(MBAP_HEADER_LEN..);

        // Step 10: return the decoded frame.
        Ok(Some(Frame {
            header: FrameHeader::Mbap(header),
            pdu,
        }))
    }
}

impl Encoder<Frame> for MbapCodec {
    type Error = FrameError;

    fn encode(&mut self, frame: Frame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let header = match frame.header {
            FrameHeader::Mbap(h) => h,
            FrameHeader::Rtu { .. } => {
                return Err(FrameError::InvalidProtocolId(0xFFFF));
            }
        };

        dst.reserve(MBAP_HEADER_LEN + frame.pdu.len());
        dst.put_slice(header.as_bytes());
        dst.put_slice(&frame.pdu);

        Ok(())
    }
}

/// Convenience [`Encoder`] implementation for `(MbapHeader, Bytes)` tuples.
///
/// This allows callers to encode a header and PDU without constructing a full [`Frame`].
impl Encoder<(MbapHeader, Bytes)> for MbapCodec {
    type Error = FrameError;

    fn encode(&mut self, item: (MbapHeader, Bytes), dst: &mut BytesMut) -> Result<(), Self::Error> {
        let (header, pdu) = item;

        dst.reserve(MBAP_HEADER_LEN + pdu.len());
        dst.put_slice(header.as_bytes());
        dst.put_slice(&pdu);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;
    use zerocopy::network_endian::U16;

    /// Build a valid MBAP frame: 7-byte header + PDU bytes.
    fn build_frame(txn_id: u16, unit_id: u8, pdu: &[u8]) -> Vec<u8> {
        let header = MbapHeader::new(txn_id, unit_id, u16::try_from(pdu.len()).unwrap());
        let mut buf = Vec::with_capacity(MBAP_HEADER_LEN + pdu.len());
        buf.extend_from_slice(header.as_bytes());
        buf.extend_from_slice(pdu);
        buf
    }

    #[test]
    fn decode_valid_frame() {
        let pdu = [0x03, 0x00, 0x00, 0x00, 0x0A]; // FC 0x03, start 0, qty 10
        let raw = build_frame(1, 0xFF, &pdu);

        let mut buf = BytesMut::from(&raw[..]);
        let mut codec = MbapCodec;

        let frame = codec
            .decode(&mut buf)
            .unwrap()
            .expect("should decode a frame");
        assert_eq!(frame.unit_id(), 0xFF);
        assert_eq!(frame.pdu.as_ref(), &pdu);
        assert!(buf.is_empty(), "buffer should be fully consumed");
    }

    #[test]
    fn decode_returns_none_on_partial_header() {
        let mut buf = BytesMut::from(&[0x00, 0x01, 0x00, 0x00][..]);
        let mut codec = MbapCodec;

        assert!(codec.decode(&mut buf).unwrap().is_none());
    }

    #[test]
    fn decode_returns_none_on_partial_body() {
        let pdu = [0x03, 0x00, 0x00, 0x00, 0x0A];
        let raw = build_frame(1, 1, &pdu);

        // Truncate 2 bytes from the end.
        let mut buf = BytesMut::from(&raw[..raw.len() - 2]);
        let mut codec = MbapCodec;

        assert!(codec.decode(&mut buf).unwrap().is_none());
    }

    #[test]
    fn decode_invalid_protocol_id() {
        let mut raw = build_frame(1, 1, &[0x03]);
        // Corrupt protocol ID to 0x0001.
        raw[2] = 0x00;
        raw[3] = 0x01;

        let mut buf = BytesMut::from(&raw[..]);
        let mut codec = MbapCodec;

        let err = codec.decode(&mut buf).unwrap_err();
        assert!(matches!(err, FrameError::InvalidProtocolId(1)));
    }

    #[test]
    fn decode_length_overflow() {
        let mut raw = build_frame(1, 1, &[0x03]);
        // Set length field to MAX_PDU_SIZE + 2 = 255 (exceeds limit of 254).
        let overflow_len = u16::try_from(MAX_PDU_SIZE).unwrap() + 2;
        raw[4] = (overflow_len >> 8) as u8;
        raw[5] = (overflow_len & 0xFF) as u8;

        let mut buf = BytesMut::from(&raw[..]);
        let mut codec = MbapCodec;

        let err = codec.decode(&mut buf).unwrap_err();
        assert!(matches!(err, FrameError::LengthOverflow(_)));
    }

    #[test]
    fn decode_multiple_frames() {
        let pdu1 = [0x03, 0x01];
        let pdu2 = [0x06, 0x02, 0x03];
        let mut raw = build_frame(1, 1, &pdu1);
        raw.extend_from_slice(&build_frame(2, 2, &pdu2));

        let mut buf = BytesMut::from(&raw[..]);
        let mut codec = MbapCodec;

        let f1 = codec.decode(&mut buf).unwrap().expect("frame 1");
        assert_eq!(f1.unit_id(), 1);
        assert_eq!(f1.pdu.as_ref(), &pdu1);

        let f2 = codec.decode(&mut buf).unwrap().expect("frame 2");
        assert_eq!(f2.unit_id(), 2);
        assert_eq!(f2.pdu.as_ref(), &pdu2);

        assert!(buf.is_empty());
    }

    #[test]
    fn encode_roundtrip() {
        let pdu = Bytes::from_static(&[0x03, 0x00, 0x00, 0x00, 0x0A]);
        let header = MbapHeader::new(42, 0xFF, u16::try_from(pdu.len()).unwrap());
        let frame = Frame {
            header: FrameHeader::Mbap(header),
            pdu: pdu.clone(),
        };

        let mut buf = BytesMut::new();
        let mut codec = MbapCodec;
        codec.encode(frame, &mut buf).unwrap();

        // Decode it back.
        let decoded = codec.decode(&mut buf).unwrap().expect("should decode");
        assert_eq!(decoded.unit_id(), 0xFF);
        assert_eq!(decoded.pdu.as_ref(), &pdu[..]);

        match decoded.header {
            FrameHeader::Mbap(h) => {
                assert_eq!(h.transaction_id.get(), 42);
            }
            FrameHeader::Rtu { .. } => panic!("expected MBAP header"),
        }
    }

    #[test]
    fn encode_tuple_form() {
        let pdu = Bytes::from_static(&[0x01, 0x00, 0x0A, 0x00, 0x0D]);
        let header = MbapHeader::new(7, 1, u16::try_from(pdu.len()).unwrap());

        let mut buf = BytesMut::new();
        let mut codec = MbapCodec;
        codec.encode((header, pdu.clone()), &mut buf).unwrap();

        // Decode it back.
        let decoded = codec.decode(&mut buf).unwrap().expect("should decode");
        assert_eq!(decoded.unit_id(), 1);
        assert_eq!(decoded.pdu.as_ref(), &pdu[..]);
    }

    #[test]
    fn encode_rtu_frame_errors() {
        let frame = Frame {
            header: FrameHeader::Rtu { unit_id: 1 },
            pdu: Bytes::from_static(&[0x03]),
        };

        let mut buf = BytesMut::new();
        let mut codec = MbapCodec;
        assert!(codec.encode(frame, &mut buf).is_err());
    }

    #[test]
    fn decode_empty_pdu() {
        // length = 1 means unit_id only, zero-length PDU.
        let header = MbapHeader {
            transaction_id: U16::new(0),
            protocol_id: U16::new(0),
            length: U16::new(1),
            unit_id: 1,
        };
        let mut buf = BytesMut::from(header.as_bytes());
        let mut codec = MbapCodec;

        let frame = codec.decode(&mut buf).unwrap().expect("should decode");
        assert!(frame.pdu.is_empty());
    }

    #[test]
    fn decode_length_zero_returns_error() {
        // length = 0 is invalid: even the unit_id byte wouldn't fit.
        // Must not panic (prior to fix, this caused an index-out-of-bounds).
        let header = MbapHeader {
            transaction_id: U16::new(0),
            protocol_id: U16::new(0),
            length: U16::new(0),
            unit_id: 1,
        };
        let mut buf = BytesMut::from(header.as_bytes());
        let mut codec = MbapCodec;

        let err = codec.decode(&mut buf).unwrap_err();
        assert!(matches!(err, FrameError::Truncated));
    }
}
