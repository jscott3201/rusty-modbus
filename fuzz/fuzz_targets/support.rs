#![forbid(unsafe_code)]
#![allow(
    dead_code,
    reason = "each fuzz binary compiles this shared module with a different helper subset"
)]

use bytes::BytesMut;
use rusty_modbus_frame::{Frame, FrameError, FrameHeader, verify_crc};
use rusty_modbus_types::MAX_PDU_SIZE;
use tokio_util::codec::{Decoder, Encoder};

pub const MAX_STREAM_INPUT: usize = 2048;
const MAX_SCHEDULE_WIDTHS: usize = 16;
const MAX_APPEND: usize = 64;

pub fn decode_incremental<D, F>(
    mut decoder: D,
    input: &[u8],
    maximum_pending: usize,
    mut inspect: F,
) where
    D: Decoder<Item = Frame, Error = FrameError>,
    F: FnMut(&Frame),
{
    let bounded = &input[..input.len().min(MAX_STREAM_INPUT)];
    if bounded.len() < 2 {
        return;
    }

    let requested_widths = usize::from(bounded[0] % MAX_SCHEDULE_WIDTHS as u8) + 1;
    let width_count = requested_widths.min(bounded.len() - 1);
    let widths = &bounded[1..=width_count];
    let stream = &bounded[1 + width_count..];
    let mut source = BytesMut::with_capacity(maximum_pending + MAX_APPEND);
    let mut offset = 0;
    let mut width_index = 0;

    while offset < stream.len() {
        let width = usize::from(widths[width_index % widths.len()]) % MAX_APPEND + 1;
        let end = (offset + width).min(stream.len());
        source.extend_from_slice(&stream[offset..end]);
        offset = end;
        width_index += 1;

        let mut calls = 0;
        // Every emitted frame must consume at least one byte. Even if the
        // maximum retained input were all emitted one byte at a time, one
        // terminal None call still fits this bound.
        let maximum_calls = maximum_pending + MAX_APPEND + 1;
        loop {
            calls += 1;
            assert!(calls <= maximum_calls);
            let before = source.len();
            match decoder.decode(&mut source) {
                Ok(Some(frame)) => {
                    assert!(source.len() < before);
                    validate_frame(&frame);
                    inspect(&frame);
                }
                Ok(None) => {
                    assert!(source.len() <= maximum_pending);
                    break;
                }
                Err(_) => return,
            }
        }
    }
}

pub fn decode_complete<D, F>(mut decoder: D, input: &[u8], maximum_input: usize, mut inspect: F)
where
    D: Decoder<Item = Frame, Error = FrameError>,
    F: FnMut(&Frame),
{
    let bounded = &input[..input.len().min(maximum_input)];
    let mut source = BytesMut::from(bounded);
    let before = source.len();
    match decoder.decode(&mut source) {
        Ok(Some(frame)) => {
            assert!(source.len() < before);
            validate_frame(&frame);
            inspect(&frame);
        }
        Ok(None) => assert!(source.len() <= maximum_input),
        Err(_) => {}
    }
}

pub fn round_trip_complete<C>(frame: &Frame, maximum_adu: usize)
where
    C: Default + Decoder<Item = Frame, Error = FrameError> + Encoder<Frame, Error = FrameError>,
{
    let mut encoded = BytesMut::with_capacity(maximum_adu);
    C::default()
        .encode(frame.clone(), &mut encoded)
        .expect("a decoded frame must encode within its framing contract");
    assert!(encoded.len() <= maximum_adu);

    let before = encoded.len();
    let decoded = C::default()
        .decode(&mut encoded)
        .expect("a freshly encoded complete frame must not fail")
        .expect("a freshly encoded complete frame must decode");
    assert!(encoded.len() < before);
    assert!(encoded.is_empty());
    assert_equivalent(frame, &decoded);
}

pub fn validate_rtu_encoding<C>(frame: &Frame, maximum_adu: usize)
where
    C: Default + Encoder<Frame, Error = FrameError>,
{
    let mut encoded = BytesMut::with_capacity(maximum_adu);
    C::default()
        .encode(frame.clone(), &mut encoded)
        .expect("a decoded RTU frame must re-encode");
    assert!(encoded.len() <= maximum_adu);
    assert!(verify_crc(&encoded));
}

fn validate_frame(frame: &Frame) {
    assert!(!frame.pdu.is_empty());
    assert!(frame.pdu.len() <= MAX_PDU_SIZE);
}

fn assert_equivalent(expected: &Frame, actual: &Frame) {
    match (expected.header, actual.header) {
        (FrameHeader::Mbap(left), FrameHeader::Mbap(right)) => {
            assert_eq!(left.transaction_id.get(), right.transaction_id.get());
            assert_eq!(left.protocol_id.get(), right.protocol_id.get());
            assert_eq!(left.length.get(), right.length.get());
            assert_eq!(left.unit_id, right.unit_id);
        }
        (FrameHeader::Rtu { unit_id: left }, FrameHeader::Rtu { unit_id: right }) => {
            assert_eq!(left, right);
        }
        _ => panic!("frame transport changed during round trip"),
    }
    assert_eq!(expected.pdu, actual.pdu);
}
