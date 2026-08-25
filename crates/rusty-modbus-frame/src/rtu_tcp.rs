//! RTU-over-TCP codec for Modbus framing.
//!
//! Carries RTU frames inside a TCP stream. This project extension is neither
//! Modbus/TCP nor physical RTU and therefore needs an explicit stream-boundary
//! policy. The unit [`RtuOverTcpCodec`] preserves the historical first-valid-CRC
//! compatibility policy. [`RtuOverTcpCodec::with_policy`] can instead select a
//! direction-aware strict policy for supported self-delimiting standard forms.
//!
//! Any decode error is terminal for the framed connection. Neither policy
//! discards bytes or attempts stream resynchronization after an error.
//!
//! # Strict length grammar
//!
//! `P` is the PDU length including the function code; RTU ADU length is `P+3`.
//! Strict framing supports these request/response lengths:
//!
//! - FC 0x01-0x04: request `P=5`; response `P=2+byte_count`.
//! - FC 0x05/0x06: `P=5`; FC 0x07: request `P=1`, response `P=2`.
//! - FC 0x08: `P=5` for fixed one-word standard sub-functions 0x0001-0x0003,
//!   0x000A-0x0012, and 0x0014. Return Query Data, Force Listen Only Mode,
//!   and unknown sub-functions are indeterminate.
//! - FC 0x0B: request `P=1`, response `P=5`; FC 0x0C: request `P=1`,
//!   response `P=2+byte_count`.
//! - FC 0x0F/0x10: request `P=6+byte_count`, response `P=5`; FC 0x11:
//!   request `P=1`, response `P=2+byte_count`.
//! - FC 0x14/0x15: `P=2+byte_count`; FC 0x16: `P=7`.
//! - FC 0x17: request `P=10+byte_count`, response `P=2+byte_count`.
//! - FC 0x18: request `P=3`, response `P=3+u16_byte_count`.
//! - FC 0x2B / MEI 0x0E: request `P=4`; response starts at `P=7` and adds
//!   `2+object_value_length` for each declared object.
//! - Exception responses use `P=2` when their base function is one of the
//!   standard functions supported above.
//!
//! Custom/reserved functions, exception-marked requests, other MEI types, and
//! any derived `P>253` are terminal strict errors. This grammar determines
//! length only; it does not validate quantities, values, or other PDU semantics.

use bytes::{BufMut, BytesMut};
use rusty_modbus_types::{MAX_PDU_SIZE, MAX_RTU_ADU_SIZE};
use tokio_util::codec::{Decoder, Encoder};

use crate::crc::{crc16, crc16_update};
use crate::error::FrameError;
use crate::frame::{Frame, FrameHeader};

/// Minimum RTU frame size: `unit_id`(1) + FC(1) + CRC(2).
const MIN_RTU_FRAME: usize = 4;
const MIN_PDU_LENGTH: usize = 1;

/// RTU-over-TCP codec.
///
/// As a bare unit value or through [`Default`], this codec uses
/// [`RtuOverTcpFramingPolicy::CrcScanCompatibility`]: candidate lengths from 4
/// through 256 bytes are tested and the first CRC-valid prefix is emitted. Use
/// [`Self::with_policy`] to make the policy and incoming direction explicit.
#[derive(Debug, Default)]
pub struct RtuOverTcpCodec;

/// Incoming RTU-over-TCP frame-boundary policy.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum RtuOverTcpFramingPolicy {
    /// Preserve the historical first-valid-CRC-prefix scanner.
    #[default]
    CrcScanCompatibility,
    /// Derive one boundary from a supported standard request or response grammar.
    FunctionAwareStrict,
}

/// Direction of incoming RTU-over-TCP frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtuOverTcpDirection {
    /// Incoming frames are requests.
    Request,
    /// Incoming frames are responses.
    Response,
}

/// RTU-over-TCP codec with an explicit incoming framing policy and direction.
#[derive(Debug, Clone, Copy)]
pub struct ConfiguredRtuOverTcpCodec {
    policy: RtuOverTcpFramingPolicy,
    direction: RtuOverTcpDirection,
}

impl RtuOverTcpCodec {
    /// Configure incoming framing.
    ///
    /// Direction is required even for compatibility mode so changing a
    /// connection to strict mode never needs to infer its role.
    #[must_use]
    pub const fn with_policy(
        policy: RtuOverTcpFramingPolicy,
        direction: RtuOverTcpDirection,
    ) -> ConfiguredRtuOverTcpCodec {
        ConfiguredRtuOverTcpCodec { policy, direction }
    }
}

impl ConfiguredRtuOverTcpCodec {
    /// Return the configured incoming framing policy.
    #[must_use]
    pub const fn policy(&self) -> RtuOverTcpFramingPolicy {
        self.policy
    }

    /// Return the configured incoming direction.
    #[must_use]
    pub const fn direction(&self) -> RtuOverTcpDirection {
        self.direction
    }
}

impl Decoder for RtuOverTcpCodec {
    type Item = Frame;
    type Error = FrameError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        decode_compatibility(src)
    }
}

impl Decoder for ConfiguredRtuOverTcpCodec {
    type Item = Frame;
    type Error = FrameError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.policy {
            RtuOverTcpFramingPolicy::CrcScanCompatibility => decode_compatibility(src),
            RtuOverTcpFramingPolicy::FunctionAwareStrict => decode_strict(src, self.direction),
        }
    }
}

impl Encoder<Frame> for RtuOverTcpCodec {
    type Error = FrameError;

    fn encode(&mut self, item: Frame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        encode_frame(&item, dst)
    }
}

impl Encoder<Frame> for ConfiguredRtuOverTcpCodec {
    type Error = FrameError;

    fn encode(&mut self, item: Frame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        encode_frame(&item, dst)
    }
}

fn decode_compatibility(src: &mut BytesMut) -> Result<Option<Frame>, FrameError> {
    if src.len() < MIN_RTU_FRAME {
        return Ok(None);
    }

    let max_len = src.len().min(MAX_RTU_ADU_SIZE);

    // Scan candidate frame lengths from smallest to largest. Keep the CRC
    // of each candidate data prefix incrementally instead of recomputing
    // it from the start for every possible frame boundary.
    let mut crc = 0xFFFF;
    crc = crc16_update(crc, src[0]);
    crc = crc16_update(crc, src[1]);
    for candidate_len in MIN_RTU_FRAME..=max_len {
        let data_end = candidate_len - 2;
        let actual = u16::from_le_bytes([src[data_end], src[data_end + 1]]);
        if crc == actual {
            return Ok(Some(split_frame(src, candidate_len)));
        }

        if candidate_len < max_len {
            crc = crc16_update(crc, src[data_end]);
        }
    }

    // No legal RTU ADU can grow past 256 bytes. At the exact bound, waiting
    // for byte 257 would retain malformed input indefinitely, so this is a
    // terminal connection error just like an over-bound buffer.
    if src.len() >= MAX_RTU_ADU_SIZE {
        return Err(FrameError::Truncated);
    }

    Ok(None)
}

fn decode_strict(
    src: &mut BytesMut,
    direction: RtuOverTcpDirection,
) -> Result<Option<Frame>, FrameError> {
    if src.len() < 2 {
        return Ok(None);
    }

    let Some(pdu_len) = strict_pdu_length(src, direction)? else {
        if src.len() >= MAX_RTU_ADU_SIZE {
            return Err(FrameError::Truncated);
        }
        return Ok(None);
    };
    validate_strict_pdu_length(pdu_len)?;
    let adu_len = pdu_len + 3;

    if src.len() < adu_len {
        return Ok(None);
    }

    let data_end = adu_len - 2;
    let expected = crc16(&src[..data_end]);
    let actual = u16::from_le_bytes([src[data_end], src[data_end + 1]]);
    if expected != actual {
        return Err(FrameError::CrcMismatch { expected, actual });
    }

    Ok(Some(split_frame(src, adu_len)))
}

fn strict_pdu_length(
    src: &[u8],
    direction: RtuOverTcpDirection,
) -> Result<Option<usize>, FrameError> {
    let function_code = src[1];
    if function_code & 0x80 != 0 {
        return match direction {
            RtuOverTcpDirection::Response
                if strict_supports_standard_base_function(function_code & 0x7F) =>
            {
                Ok(Some(2))
            }
            RtuOverTcpDirection::Request | RtuOverTcpDirection::Response => {
                Err(indeterminate(function_code))
            }
        };
    }

    match (function_code, direction) {
        (0x01..=0x04, RtuOverTcpDirection::Request)
        | (0x05 | 0x06, _)
        | (0x0B | 0x0F | 0x10, RtuOverTcpDirection::Response) => Ok(Some(5)),
        (0x01..=0x04 | 0x0C | 0x11 | 0x17, RtuOverTcpDirection::Response) | (0x14 | 0x15, _) => {
            pdu_length_from_u8_count(src, 2, 2)
        }
        (0x07 | 0x0B | 0x0C | 0x11, RtuOverTcpDirection::Request) => Ok(Some(1)),
        (0x07, RtuOverTcpDirection::Response) => Ok(Some(2)),
        (0x08, _) => diagnostic_pdu_length(src),
        (0x0F | 0x10, RtuOverTcpDirection::Request) => pdu_length_from_u8_count(src, 6, 6),
        (0x16, _) => Ok(Some(7)),
        (0x17, RtuOverTcpDirection::Request) => pdu_length_from_u8_count(src, 10, 10),
        (0x18, RtuOverTcpDirection::Request) => Ok(Some(3)),
        (0x18, RtuOverTcpDirection::Response) => pdu_length_from_u16_count(src, 3, 2),
        (0x2B, RtuOverTcpDirection::Request) => mei_request_pdu_length(src),
        (0x2B, RtuOverTcpDirection::Response) => mei_response_pdu_length(src),
        _ => Err(indeterminate(function_code)),
    }
}

const fn strict_supports_standard_base_function(function_code: u8) -> bool {
    matches!(
        function_code,
        0x01..=0x08
            | 0x0B
            | 0x0C
            | 0x0F..=0x11
            | 0x14..=0x18
            | 0x2B
    )
}

fn pdu_length_from_u8_count(
    src: &[u8],
    base_pdu_len: usize,
    count_adu_index: usize,
) -> Result<Option<usize>, FrameError> {
    let Some(&count) = src.get(count_adu_index) else {
        return Ok(None);
    };
    let length = base_pdu_len + usize::from(count);
    validate_strict_pdu_length(length)?;
    Ok(Some(length))
}

fn pdu_length_from_u16_count(
    src: &[u8],
    base_pdu_len: usize,
    count_adu_index: usize,
) -> Result<Option<usize>, FrameError> {
    let Some(bytes) = src.get(count_adu_index..count_adu_index + 2) else {
        return Ok(None);
    };
    let count = usize::from(u16::from_be_bytes([bytes[0], bytes[1]]));
    let length = base_pdu_len + count;
    validate_strict_pdu_length(length)?;
    Ok(Some(length))
}

fn diagnostic_pdu_length(src: &[u8]) -> Result<Option<usize>, FrameError> {
    let Some(bytes) = src.get(2..4) else {
        return Ok(None);
    };
    let sub_function = u16::from_be_bytes([bytes[0], bytes[1]]);

    // Return Query Data (0x0000) is variable. Force Listen Only Mode (0x0004)
    // has no normal response. Every other sub-function represented by the
    // current standard type has exactly one two-byte data word.
    match sub_function {
        0x0001..=0x0003 | 0x000A..=0x0012 | 0x0014 => Ok(Some(5)),
        _ => Err(indeterminate(0x08)),
    }
}

fn mei_request_pdu_length(src: &[u8]) -> Result<Option<usize>, FrameError> {
    let Some(&mei_type) = src.get(2) else {
        return Ok(None);
    };
    if mei_type == 0x0E {
        Ok(Some(4))
    } else {
        Err(indeterminate(0x2B))
    }
}

fn mei_response_pdu_length(src: &[u8]) -> Result<Option<usize>, FrameError> {
    let Some(&mei_type) = src.get(2) else {
        return Ok(None);
    };
    if mei_type != 0x0E {
        return Err(indeterminate(0x2B));
    }
    let Some(&object_count) = src.get(7) else {
        return Ok(None);
    };

    let mut pdu_len = 7usize;
    let minimum = pdu_len + usize::from(object_count) * 2;
    validate_strict_pdu_length(minimum)?;

    for _ in 0..object_count {
        let object_adu_index = 1 + pdu_len;
        let Some(header) = src.get(object_adu_index..object_adu_index + 2) else {
            return Ok(None);
        };
        pdu_len += 2 + usize::from(header[1]);
        validate_strict_pdu_length(pdu_len)?;
    }

    Ok(Some(pdu_len))
}

fn validate_strict_pdu_length(length: usize) -> Result<(), FrameError> {
    if length > MAX_PDU_SIZE {
        return Err(FrameError::InvalidRtuOverTcpFrameLength {
            length,
            maximum: MAX_PDU_SIZE,
        });
    }
    Ok(())
}

const fn indeterminate(function_code: u8) -> FrameError {
    FrameError::IndeterminateRtuOverTcpFrameLength { function_code }
}

fn split_frame(src: &mut BytesMut, adu_len: usize) -> Frame {
    let unit_id = src[0];
    let adu = src.split_to(adu_len).freeze();
    let pdu = adu.slice(1..adu.len() - 2);
    Frame {
        header: FrameHeader::Rtu { unit_id },
        pdu,
    }
}

fn encode_frame(item: &Frame, dst: &mut BytesMut) -> Result<(), FrameError> {
    let unit_id = match item.header {
        FrameHeader::Rtu { unit_id } => unit_id,
        FrameHeader::Mbap(h) => h.unit_id,
    };
    validate_outgoing_pdu(item.pdu.len())?;

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
    use crate::crc::verify_crc;

    /// Build a valid RTU frame: [`unit_id`, pdu..., `crc_lo`, `crc_hi`].
    fn make_rtu_frame(unit_id: u8, pdu: &[u8]) -> Vec<u8> {
        let mut buf = vec![unit_id];
        buf.extend_from_slice(pdu);
        let crc = crc16(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());
        buf
    }

    fn strict(direction: RtuOverTcpDirection) -> ConfiguredRtuOverTcpCodec {
        RtuOverTcpCodec::with_policy(RtuOverTcpFramingPolicy::FunctionAwareStrict, direction)
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
    fn encode_rejects_empty_pdu() {
        let frame = Frame {
            header: FrameHeader::Rtu { unit_id: 0x01 },
            pdu: bytes::Bytes::new(),
        };

        let mut dst = BytesMut::new();
        let mut codec = RtuOverTcpCodec;

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
        let mut codec = RtuOverTcpCodec;

        let err = codec.encode(frame, &mut dst).unwrap_err();
        assert!(matches!(err, FrameError::PduLengthOverflow { .. }));
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

    #[test]
    fn exact_max_len_crc_miss_is_terminal() {
        let raw = crc_miss_buffer(MAX_RTU_ADU_SIZE);
        let mut buf = BytesMut::from(&raw[..]);
        let mut codec = RtuOverTcpCodec;

        assert!(matches!(codec.decode(&mut buf), Err(FrameError::Truncated)));
        assert_eq!(buf.len(), MAX_RTU_ADU_SIZE);
    }

    #[test]
    fn configured_compatibility_preserves_first_valid_prefix() {
        let raw = frame_with_early_crc_prefix();
        let first_prefix = (MIN_RTU_FRAME..raw.len())
            .find(|&len| verify_crc(&raw[..len]))
            .unwrap();
        let mut buf = BytesMut::from(&raw[..]);
        let mut codec = RtuOverTcpCodec::with_policy(
            RtuOverTcpFramingPolicy::CrcScanCompatibility,
            RtuOverTcpDirection::Response,
        );

        let frame = codec.decode(&mut buf).unwrap().unwrap();

        assert_eq!(frame.pdu.len(), first_prefix - 3);
        assert_eq!(buf.len(), raw.len() - first_prefix);
    }

    #[test]
    fn strict_ignores_false_early_crc_prefix() {
        let raw = frame_with_early_crc_prefix();
        assert!((MIN_RTU_FRAME..raw.len()).any(|len| verify_crc(&raw[..len])));
        let mut buf = BytesMut::from(&raw[..]);
        let mut codec = strict(RtuOverTcpDirection::Response);

        let frame = codec.decode(&mut buf).unwrap().unwrap();

        assert_eq!(&frame.pdu[..], &raw[1..raw.len() - 2]);
        assert!(buf.is_empty());
    }

    #[test]
    fn strict_supported_request_forms_have_one_boundary() {
        let forms: &[&[u8]] = &[
            &[0x01, 0, 0, 0, 1],
            &[0x02, 0, 0, 0, 1],
            &[0x03, 0, 0, 0, 1],
            &[0x04, 0, 0, 0, 1],
            &[0x05, 0, 0, 0xFF, 0],
            &[0x06, 0, 0, 0, 1],
            &[0x07],
            &[0x08, 0, 1, 0, 0],
            &[0x0B],
            &[0x0C],
            &[0x0F, 0, 0, 0, 8, 1, 0],
            &[0x10, 0, 0, 0, 1, 2, 0, 1],
            &[0x11],
            &[0x14, 3, 0xAA, 0xBB, 0xCC],
            &[0x15, 2, 0xAA, 0xBB],
            &[0x16, 0, 1, 0xFF, 0, 0, 1],
            &[0x17, 0, 0, 0, 1, 0, 2, 0, 1, 2, 0, 3],
            &[0x18, 0, 1],
            &[0x2B, 0x0E, 1, 0],
        ];

        for &pdu in forms {
            assert_strict_decodes(RtuOverTcpDirection::Request, pdu);
        }
    }

    #[test]
    fn strict_supported_response_forms_have_one_boundary() {
        let forms: &[&[u8]] = &[
            &[0x01, 1, 0],
            &[0x02, 1, 0],
            &[0x03, 2, 0, 1],
            &[0x04, 2, 0, 1],
            &[0x05, 0, 0, 0xFF, 0],
            &[0x06, 0, 0, 0, 1],
            &[0x07, 0],
            &[0x08, 0, 2, 0, 0],
            &[0x0B, 0, 0, 0, 1],
            &[0x0C, 6, 0, 0, 0, 1, 0, 1],
            &[0x0F, 0, 0, 0, 8],
            &[0x10, 0, 0, 0, 1],
            &[0x11, 2, 0xAA, 0xBB],
            &[0x14, 2, 0xAA, 0xBB],
            &[0x15, 2, 0xAA, 0xBB],
            &[0x16, 0, 1, 0xFF, 0, 0, 1],
            &[0x17, 2, 0, 1],
            &[0x18, 0, 2, 0, 1],
            &[
                0x2B, 0x0E, 1, 1, 0, 0, 2, 0, 3, b'a', b'b', b'c', 1, 1, b'z',
            ],
            &[0x83, 2],
        ];

        for &pdu in forms {
            assert_strict_decodes(RtuOverTcpDirection::Response, pdu);
        }
    }

    #[test]
    fn strict_supports_each_fixed_word_diagnostic_sub_function() {
        let fixed_sub_functions: [u16; 13] = [
            0x0001, 0x0002, 0x0003, 0x000A, 0x000B, 0x000C, 0x000D, 0x000E, 0x000F, 0x0010, 0x0011,
            0x0012, 0x0014,
        ];

        for sub_function in fixed_sub_functions {
            let [high, low] = sub_function.to_be_bytes();
            let pdu = [0x08, high, low, 0x12, 0x34];
            assert_strict_decodes(RtuOverTcpDirection::Request, &pdu);
            assert_strict_decodes(RtuOverTcpDirection::Response, &pdu);
        }
    }

    #[test]
    fn strict_rejects_custom_exception_while_compatibility_accepts_it() {
        let raw = make_rtu_frame(1, &[0xE5, 2]);
        let mut strict_source = BytesMut::from(&raw[..]);

        assert!(matches!(
            strict(RtuOverTcpDirection::Response).decode(&mut strict_source),
            Err(FrameError::IndeterminateRtuOverTcpFrameLength {
                function_code: 0xE5
            })
        ));
        assert_eq!(&strict_source[..], &raw);

        let mut compatibility_source = BytesMut::from(&raw[..]);
        let frame = RtuOverTcpCodec
            .decode(&mut compatibility_source)
            .unwrap()
            .unwrap();
        assert_eq!(&frame.pdu[..], &[0xE5, 2]);
        assert!(compatibility_source.is_empty());
    }

    #[test]
    fn strict_fragments_and_leaves_coalesced_frame_buffered() {
        let first = make_rtu_frame(1, &[0x03, 2, 0, 1]);
        let second = make_rtu_frame(2, &[0x07, 0xA5]);
        let mut buf = BytesMut::from(&first[..first.len() - 1]);
        let mut codec = strict(RtuOverTcpDirection::Response);

        assert!(codec.decode(&mut buf).unwrap().is_none());
        buf.extend_from_slice(&first[first.len() - 1..]);
        buf.extend_from_slice(&second);

        let decoded_first = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded_first.unit_id(), 1);
        assert_eq!(&buf[..], &second);
        let decoded_second = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded_second.unit_id(), 2);
        assert!(buf.is_empty());
    }

    #[test]
    fn strict_accepts_maximum_adu_and_framing_only_invalid_values() {
        let mut maximum_pdu = vec![0x14, 251];
        maximum_pdu.resize(MAX_PDU_SIZE, 0xA5);
        assert_strict_decodes(RtuOverTcpDirection::Response, &maximum_pdu);

        // Odd FC03 byte counts are semantically invalid but still self-delimiting.
        assert_strict_decodes(RtuOverTcpDirection::Response, &[0x03, 1, 0xAA]);
        // Quantity and byte-count agreement is likewise outside framing.
        assert_strict_decodes(RtuOverTcpDirection::Request, &[0x10, 0, 0, 0, 123, 2, 0, 1]);
    }

    #[test]
    fn strict_rejects_declared_overflow_and_bad_boundary_crc() {
        let mut overflow = BytesMut::from(&[1, 0x14, 252][..]);
        let err = strict(RtuOverTcpDirection::Response)
            .decode(&mut overflow)
            .unwrap_err();
        assert!(matches!(
            err,
            FrameError::InvalidRtuOverTcpFrameLength {
                length: 254,
                maximum: MAX_PDU_SIZE
            }
        ));

        let mut u16_overflow = BytesMut::from(&[1, 0x18, 0, 251][..]);
        assert!(matches!(
            strict(RtuOverTcpDirection::Response).decode(&mut u16_overflow),
            Err(FrameError::InvalidRtuOverTcpFrameLength { length: 254, .. })
        ));

        let mut mei_overflow = BytesMut::from(&[1, 0x2B, 0x0E, 1, 1, 0, 0, 1, 0, 245][..]);
        assert!(matches!(
            strict(RtuOverTcpDirection::Response).decode(&mut mei_overflow),
            Err(FrameError::InvalidRtuOverTcpFrameLength { length: 254, .. })
        ));

        let mut raw = make_rtu_frame(1, &[0x03, 2, 0, 1]);
        let last = raw.len() - 1;
        raw[last] ^= 0xFF;
        let mut corrupt = BytesMut::from(&raw[..]);
        assert!(matches!(
            strict(RtuOverTcpDirection::Response).decode(&mut corrupt),
            Err(FrameError::CrcMismatch { .. })
        ));
    }

    #[test]
    fn strict_rejects_indeterminate_grammar_without_crc_scanning() {
        let cases: &[(RtuOverTcpDirection, &[u8], u8)] = &[
            (RtuOverTcpDirection::Request, &[0x41, 0, 0], 0x41),
            (RtuOverTcpDirection::Response, &[0x41, 0, 0], 0x41),
            (RtuOverTcpDirection::Request, &[0x83, 2], 0x83),
            (RtuOverTcpDirection::Request, &[0x08, 0, 0, 0, 1], 0x08),
            (RtuOverTcpDirection::Response, &[0x08, 0, 4, 0, 0], 0x08),
            (RtuOverTcpDirection::Request, &[0x08, 0, 5, 0, 0], 0x08),
            (RtuOverTcpDirection::Request, &[0x2B, 0x0D, 0, 0], 0x2B),
            (RtuOverTcpDirection::Response, &[0x2B, 0x0D, 0], 0x2B),
        ];

        for &(direction, pdu, function_code) in cases {
            let raw = make_rtu_frame(1, pdu);
            let mut buf = BytesMut::from(&raw[..]);
            assert!(matches!(
                strict(direction).decode(&mut buf),
                Err(FrameError::IndeterminateRtuOverTcpFrameLength {
                    function_code: actual
                }) if actual == function_code
            ));
        }
    }

    #[test]
    fn strict_exact_max_bad_crc_is_terminal() {
        let mut pdu = vec![0x14, 251];
        pdu.resize(MAX_PDU_SIZE, 0x5A);
        let mut raw = make_rtu_frame(1, &pdu);
        assert_eq!(raw.len(), MAX_RTU_ADU_SIZE);
        let last = raw.len() - 1;
        raw[last] ^= 0x01;
        let mut buf = BytesMut::from(&raw[..]);

        assert!(matches!(
            strict(RtuOverTcpDirection::Response).decode(&mut buf),
            Err(FrameError::CrcMismatch { .. })
        ));
    }

    fn assert_strict_decodes(direction: RtuOverTcpDirection, pdu: &[u8]) {
        let raw = make_rtu_frame(1, pdu);
        let mut buf = BytesMut::from(&raw[..]);
        let frame = strict(direction).decode(&mut buf).unwrap().unwrap();
        assert_eq!(&frame.pdu[..], pdu, "strict form {pdu:02X?}");
        assert!(buf.is_empty());
    }

    fn frame_with_early_crc_prefix() -> Vec<u8> {
        for value in 0..=u16::MAX {
            let [high, low] = value.to_be_bytes();
            let raw = make_rtu_frame(1, &[0x03, 4, high, low, 0x12, 0x34]);
            if (MIN_RTU_FRAME..raw.len()).any(|len| verify_crc(&raw[..len])) {
                return raw;
            }
        }
        unreachable!("the bounded search contains an early CRC-valid prefix");
    }

    fn crc_miss_buffer(len: usize) -> Vec<u8> {
        for salt in 0u8..=u8::MAX {
            let candidate: Vec<u8> = (0..len)
                .map(|i| {
                    let byte = u8::try_from(i % 251).expect("modulo 251 fits u8");
                    byte.wrapping_mul(37).wrapping_add(0xA5 ^ salt)
                })
                .collect();
            if (MIN_RTU_FRAME..=len).all(|candidate_len| !verify_crc(&candidate[..candidate_len])) {
                return candidate;
            }
        }
        unreachable!("salted deterministic buffers should produce a CRC-miss case");
    }
}
