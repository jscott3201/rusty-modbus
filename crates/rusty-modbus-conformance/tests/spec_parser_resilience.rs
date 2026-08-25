//! Fixed-seed parser properties and named malformed-input vectors.

#![forbid(unsafe_code)]

use bytes::{Bytes, BytesMut};
use proptest::collection;
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, RngSeed, TestCaseError};
use rusty_modbus_codec::{
    DecodeError, RequestPdu, ResponsePdu, decode_pdu_ref, decode_request, decode_response,
};
use rusty_modbus_frame::crc::{crc16, verify_crc};
use rusty_modbus_frame::rtu::RtuCodec;
use rusty_modbus_frame::rtu_tcp::RtuOverTcpCodec;
use rusty_modbus_frame::{Frame, FrameError, FrameHeader, MbapCodec};
use rusty_modbus_types::{
    FunctionCode, MAX_PDU_SIZE, MAX_READ_REGISTERS, MAX_RTU_ADU_SIZE, MAX_TCP_ADU_SIZE,
    MBAP_HEADER_LEN, MbapHeader,
};
use tokio_util::codec::{Decoder, Encoder};
use zerocopy::IntoBytes;

const PROPERTY_CASES: u32 = 128;
const PROPERTY_SEED: u64 = 0x5EED_0030_2026_0823;

fn property_config() -> Config {
    Config {
        cases: PROPERTY_CASES,
        failure_persistence: None,
        max_shrink_iters: 4096,
        rng_algorithm: RngAlgorithm::ChaCha,
        rng_seed: RngSeed::Fixed(PROPERTY_SEED),
        ..Config::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ObservedHeader {
    Mbap {
        transaction_id: u16,
        protocol_id: u16,
        length: u16,
        unit_id: u8,
    },
    Rtu {
        unit_id: u8,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ObservedFrame {
    header: ObservedHeader,
    pdu: Vec<u8>,
}

fn observe(frame: Frame) -> ObservedFrame {
    let header = match frame.header {
        FrameHeader::Mbap(header) => ObservedHeader::Mbap {
            transaction_id: header.transaction_id.get(),
            protocol_id: header.protocol_id.get(),
            length: header.length.get(),
            unit_id: header.unit_id,
        },
        FrameHeader::Rtu { unit_id } => ObservedHeader::Rtu { unit_id },
    };
    ObservedFrame {
        header,
        pdu: frame.pdu.to_vec(),
    }
}

fn decode_with_schedule<D>(
    mut decoder: D,
    stream: &[u8],
    schedule: &[usize],
    maximum_pending: usize,
) -> Result<(Vec<ObservedFrame>, usize), String>
where
    D: Decoder<Item = Frame, Error = FrameError>,
{
    if schedule.is_empty() || schedule.contains(&0) {
        return Err("chunk schedule must contain nonzero widths".to_owned());
    }

    let mut frames = Vec::new();
    let mut source = BytesMut::new();
    let mut offset = 0;
    let mut schedule_index = 0;

    while offset < stream.len() {
        let end = (offset + schedule[schedule_index % schedule.len()]).min(stream.len());
        source.extend_from_slice(&stream[offset..end]);
        offset = end;
        schedule_index += 1;

        loop {
            match decoder
                .decode(&mut source)
                .map_err(|error| error.to_string())?
            {
                Some(frame) => frames.push(observe(frame)),
                None => {
                    if source.len() > maximum_pending {
                        return Err(format!(
                            "decoder retained {} bytes above the {}-byte protocol bound",
                            source.len(),
                            maximum_pending
                        ));
                    }
                    break;
                }
            }
        }
    }

    Ok((frames, source.len()))
}

fn build_mbap_adu(transaction_id: u16, unit_id: u8, pdu: &[u8]) -> Vec<u8> {
    let pdu_length = u16::try_from(pdu.len()).expect("bounded PDU length fits u16");
    let header = MbapHeader::new(transaction_id, unit_id, pdu_length);
    let mut adu = Vec::with_capacity(MBAP_HEADER_LEN + pdu.len());
    adu.extend_from_slice(header.as_bytes());
    adu.extend_from_slice(pdu);
    adu
}

fn build_rtu_adu(unit_id: u8, pdu: &[u8]) -> Vec<u8> {
    let mut adu = Vec::with_capacity(1 + pdu.len() + 2);
    adu.push(unit_id);
    adu.extend_from_slice(pdu);
    adu.extend_from_slice(&crc16(&adu).to_le_bytes());
    adu
}

fn has_only_terminal_crc(adu: &[u8]) -> bool {
    verify_crc(adu) && (4..adu.len()).all(|candidate_len| !verify_crc(&adu[..candidate_len]))
}

fn build_unambiguous_rtu_adu(unit_id: u8, pdu: &[u8]) -> Vec<u8> {
    for salt in 0..=u8::MAX {
        let mut adjusted = pdu.to_vec();
        adjusted[0] = adjusted[0].wrapping_add(salt);
        let adu = build_rtu_adu(unit_id, &adjusted);
        if has_only_terminal_crc(&adu) {
            return adu;
        }
    }
    unreachable!("a one-byte salt should avoid an earlier CRC-valid prefix")
}

fn crc_miss_buffer(length: usize) -> Vec<u8> {
    for salt in 0..=u8::MAX {
        let candidate: Vec<u8> = (0..length)
            .map(|index| {
                let byte = u8::try_from(index % 251).expect("modulo 251 fits u8");
                byte.wrapping_mul(37).wrapping_add(0xA5 ^ salt)
            })
            .collect();
        if (4..=length).all(|candidate_len| !verify_crc(&candidate[..candidate_len])) {
            return candidate;
        }
    }
    unreachable!("a one-byte salt should produce a CRC-miss buffer")
}

fn request_function_code(request: &RequestPdu<'_>) -> u8 {
    match request {
        RequestPdu::ReadCoils(_) => FunctionCode::ReadCoils.code(),
        RequestPdu::ReadDiscreteInputs(_) => FunctionCode::ReadDiscreteInputs.code(),
        RequestPdu::ReadHoldingRegisters(_) => FunctionCode::ReadHoldingRegisters.code(),
        RequestPdu::ReadInputRegisters(_) => FunctionCode::ReadInputRegisters.code(),
        RequestPdu::WriteSingleCoil(_) => FunctionCode::WriteSingleCoil.code(),
        RequestPdu::WriteSingleRegister(_) => FunctionCode::WriteSingleRegister.code(),
        RequestPdu::ReadExceptionStatus => FunctionCode::ReadExceptionStatus.code(),
        RequestPdu::Diagnostics(_) => FunctionCode::Diagnostics.code(),
        RequestPdu::GetCommEventCounter => FunctionCode::GetCommEventCounter.code(),
        RequestPdu::GetCommEventLog => FunctionCode::GetCommEventLog.code(),
        RequestPdu::WriteMultipleCoils(_) => FunctionCode::WriteMultipleCoils.code(),
        RequestPdu::WriteMultipleRegisters(_) => FunctionCode::WriteMultipleRegisters.code(),
        RequestPdu::ReportServerId => FunctionCode::ReportServerId.code(),
        RequestPdu::ReadFileRecord(_) => FunctionCode::ReadFileRecord.code(),
        RequestPdu::WriteFileRecord(_) => FunctionCode::WriteFileRecord.code(),
        RequestPdu::MaskWriteRegister(_) => FunctionCode::MaskWriteRegister.code(),
        RequestPdu::ReadWriteMultipleRegisters(_) => {
            FunctionCode::ReadWriteMultipleRegisters.code()
        }
        RequestPdu::ReadFifoQueue(_) => FunctionCode::ReadFifoQueue.code(),
        RequestPdu::EncapsulatedInterface(_) => FunctionCode::EncapsulatedInterfaceTransport.code(),
        RequestPdu::Custom(function_code, _) => *function_code,
    }
}

fn response_function_code(response: &ResponsePdu<'_>) -> u8 {
    match response {
        ResponsePdu::ReadCoils(_) => FunctionCode::ReadCoils.code(),
        ResponsePdu::ReadDiscreteInputs(_) => FunctionCode::ReadDiscreteInputs.code(),
        ResponsePdu::ReadHoldingRegisters(_) => FunctionCode::ReadHoldingRegisters.code(),
        ResponsePdu::ReadInputRegisters(_) => FunctionCode::ReadInputRegisters.code(),
        ResponsePdu::WriteSingleCoil(_) => FunctionCode::WriteSingleCoil.code(),
        ResponsePdu::WriteSingleRegister(_) => FunctionCode::WriteSingleRegister.code(),
        ResponsePdu::ReadExceptionStatus(_) => FunctionCode::ReadExceptionStatus.code(),
        ResponsePdu::Diagnostics(_) => FunctionCode::Diagnostics.code(),
        ResponsePdu::GetCommEventCounter(_) => FunctionCode::GetCommEventCounter.code(),
        ResponsePdu::GetCommEventLog(_) => FunctionCode::GetCommEventLog.code(),
        ResponsePdu::WriteMultipleCoils(_) => FunctionCode::WriteMultipleCoils.code(),
        ResponsePdu::WriteMultipleRegisters(_) => FunctionCode::WriteMultipleRegisters.code(),
        ResponsePdu::ReportServerId(_) => FunctionCode::ReportServerId.code(),
        ResponsePdu::ReadFileRecord(_) => FunctionCode::ReadFileRecord.code(),
        ResponsePdu::WriteFileRecord(_) => FunctionCode::WriteFileRecord.code(),
        ResponsePdu::MaskWriteRegister(_) => FunctionCode::MaskWriteRegister.code(),
        ResponsePdu::ReadWriteMultipleRegisters(_) => {
            FunctionCode::ReadWriteMultipleRegisters.code()
        }
        ResponsePdu::ReadFifoQueue(_) => FunctionCode::ReadFifoQueue.code(),
        ResponsePdu::EncapsulatedInterface(_) => {
            FunctionCode::EncapsulatedInterfaceTransport.code()
        }
        ResponsePdu::Custom(function_code, _) => *function_code,
        ResponsePdu::Exception(exception) => exception.function_code.exception_code(),
    }
}

fn property_failure(error: String) -> TestCaseError {
    TestCaseError::fail(error)
}

proptest! {
    #![proptest_config(property_config())]

    #[test]
    fn bounded_raw_pdus_have_consistent_top_level_dispatch(
        pdu in collection::vec(any::<u8>(), 0..=(MAX_PDU_SIZE + 8)),
    ) {
        let raw = decode_pdu_ref(&pdu);
        let request = decode_request(&pdu);
        let response = decode_response(&pdu);

        if pdu.is_empty() {
            let expected = DecodeError::Truncated { expected: 1, actual: 0 };
            prop_assert_eq!(raw.unwrap_err(), expected);
            prop_assert_eq!(request.unwrap_err(), expected);
            prop_assert_eq!(response.unwrap_err(), expected);
        } else if pdu.len() > MAX_PDU_SIZE {
            let expected = DecodeError::PduTooLarge {
                length: pdu.len(),
                maximum: MAX_PDU_SIZE,
            };
            prop_assert_eq!(raw.unwrap_err(), expected);
            prop_assert_eq!(request.unwrap_err(), expected);
            prop_assert_eq!(response.unwrap_err(), expected);
        } else {
            let raw = raw.expect("nonempty bounded PDU has a raw view");
            prop_assert_eq!(raw.function_code, pdu[0]);
            prop_assert_eq!(raw.data, &pdu[1..]);
            if let Ok(request) = request {
                prop_assert_eq!(request_function_code(&request), pdu[0]);
            }
            if let Ok(response) = response {
                prop_assert_eq!(response_function_code(&response), pdu[0]);
            }
        }
    }

    #[test]
    fn generated_fc03_requests_preserve_big_endian_fields_and_reject_near_misses(
        address in any::<u16>(),
        quantity in 1u16..=MAX_READ_REGISTERS,
        trailing in any::<u8>(),
    ) {
        let mut pdu = vec![FunctionCode::ReadHoldingRegisters.code()];
        pdu.extend_from_slice(&address.to_be_bytes());
        pdu.extend_from_slice(&quantity.to_be_bytes());

        match decode_request(&pdu).expect("generated FC03 request is valid") {
            RequestPdu::ReadHoldingRegisters(request) => {
                prop_assert_eq!(request.address.0, address);
                prop_assert_eq!(request.quantity.0, quantity);
            }
            other => prop_assert!(false, "unexpected request dispatch: {other:?}"),
        }

        prop_assert_eq!(
            decode_request(&pdu[..pdu.len() - 1]).unwrap_err(),
            DecodeError::Truncated { expected: 4, actual: 3 },
        );
        pdu.push(trailing);
        prop_assert_eq!(
            decode_request(&pdu).unwrap_err(),
            DecodeError::LengthMismatch { expected: 4, actual: 5 },
        );
    }

    #[test]
    fn generated_fc03_responses_preserve_register_bytes_and_reject_bad_counts(
        registers in collection::vec(any::<u16>(), 1..=usize::from(MAX_READ_REGISTERS)),
    ) {
        let mut pdu = Vec::with_capacity(2 + registers.len() * 2);
        pdu.push(FunctionCode::ReadHoldingRegisters.code());
        pdu.push(u8::try_from(registers.len() * 2).expect("FC03 response byte count fits u8"));
        for register in &registers {
            pdu.extend_from_slice(&register.to_be_bytes());
        }

        match decode_response(&pdu).expect("generated FC03 response is valid") {
            ResponsePdu::ReadHoldingRegisters(response) => {
                prop_assert_eq!(usize::from(response.byte_count), registers.len() * 2);
                prop_assert_eq!(response.register_data, &pdu[2..]);
                for (index, expected) in registers.iter().copied().enumerate() {
                    prop_assert_eq!(response.register(index), expected);
                }
            }
            other => prop_assert!(false, "unexpected response dispatch: {other:?}"),
        }

        pdu[1] += 1;
        prop_assert_eq!(
            decode_response(&pdu).unwrap_err(),
            DecodeError::ByteCountMismatch {
                declared: registers.len() * 2 + 1,
                actual: registers.len() * 2,
            },
        );
    }

    #[test]
    fn mbap_frames_are_invariant_across_bounded_chunk_schedules(
        cases in collection::vec(
            (
                any::<u16>(),
                any::<u8>(),
                collection::vec(any::<u8>(), 1..=MAX_PDU_SIZE),
            ),
            1..=3,
        ),
        generated_schedule in collection::vec(1usize..=MAX_TCP_ADU_SIZE, 1..=32),
    ) {
        let mut stream = Vec::new();
        let mut frame_lengths = Vec::new();
        let mut expected = Vec::new();
        for (transaction_id, unit_id, pdu) in &cases {
            let adu = build_mbap_adu(*transaction_id, *unit_id, pdu);
            frame_lengths.push(adu.len());
            stream.extend_from_slice(&adu);
            expected.push(ObservedFrame {
                header: ObservedHeader::Mbap {
                    transaction_id: *transaction_id,
                    protocol_id: 0,
                    length: u16::try_from(pdu.len() + 1).expect("bounded MBAP length fits u16"),
                    unit_id: *unit_id,
                },
                pdu: pdu.clone(),
            });
        }

        let (unchunked, pending) = decode_with_schedule(
            MbapCodec,
            &stream,
            &[stream.len()],
            MAX_TCP_ADU_SIZE,
        ).map_err(property_failure)?;
        prop_assert_eq!(pending, 0);
        prop_assert_eq!(&unchunked, &expected);

        let one_byte = [1];
        let header_boundary = [MBAP_HEADER_LEN - 1, 1, 1, 17, MAX_TCP_ADU_SIZE];
        for schedule in [
            one_byte.as_slice(),
            header_boundary.as_slice(),
            frame_lengths.as_slice(),
            generated_schedule.as_slice(),
        ] {
            let (observed, pending) = decode_with_schedule(
                MbapCodec,
                &stream,
                schedule,
                MAX_TCP_ADU_SIZE,
            ).map_err(property_failure)?;
            prop_assert_eq!(pending, 0);
            prop_assert_eq!(&observed, &unchunked);
        }
    }

    #[test]
    fn rtu_over_tcp_frames_are_invariant_across_bounded_chunk_schedules(
        cases in collection::vec(
            (any::<u8>(), collection::vec(any::<u8>(), 1..=MAX_PDU_SIZE)),
            1..=3,
        ),
        generated_schedule in collection::vec(1usize..=MAX_RTU_ADU_SIZE, 1..=32),
    ) {
        let mut stream = Vec::new();
        let mut frame_lengths = Vec::new();
        let mut expected = Vec::new();
        for (unit_id, pdu) in &cases {
            let adu = build_unambiguous_rtu_adu(*unit_id, pdu);
            frame_lengths.push(adu.len());
            expected.push(ObservedFrame {
                header: ObservedHeader::Rtu { unit_id: adu[0] },
                pdu: adu[1..adu.len() - 2].to_vec(),
            });
            stream.extend_from_slice(&adu);
        }

        let (unchunked, pending) = decode_with_schedule(
            RtuOverTcpCodec,
            &stream,
            &[stream.len()],
            MAX_RTU_ADU_SIZE,
        ).map_err(property_failure)?;
        prop_assert_eq!(pending, 0);
        prop_assert_eq!(&unchunked, &expected);

        let one_byte = [1];
        let minimum_boundary = [3, 1, 1, 19, MAX_RTU_ADU_SIZE];
        for schedule in [
            one_byte.as_slice(),
            minimum_boundary.as_slice(),
            frame_lengths.as_slice(),
            generated_schedule.as_slice(),
        ] {
            let (observed, pending) = decode_with_schedule(
                RtuOverTcpCodec,
                &stream,
                schedule,
                MAX_RTU_ADU_SIZE,
            ).map_err(property_failure)?;
            prop_assert_eq!(pending, 0);
            prop_assert_eq!(&observed, &unchunked);
        }
    }

    #[test]
    fn complete_physical_rtu_frames_round_trip_within_the_adu_bound(
        unit_id in any::<u8>(),
        pdu in collection::vec(any::<u8>(), 1..=MAX_PDU_SIZE),
    ) {
        let frame = Frame {
            header: FrameHeader::Rtu { unit_id },
            pdu: Bytes::copy_from_slice(&pdu),
        };
        let mut source = BytesMut::new();
        let mut codec = RtuCodec;
        codec.encode(frame, &mut source).expect("bounded RTU frame encodes");
        prop_assert_eq!(source.len(), pdu.len() + 3);
        prop_assert!(source.len() <= MAX_RTU_ADU_SIZE);

        let decoded = codec
            .decode(&mut source)
            .expect("complete generated RTU frame has a valid CRC")
            .expect("complete generated RTU frame decodes");
        prop_assert_eq!(decoded.unit_id(), unit_id);
        prop_assert_eq!(decoded.pdu.as_ref(), pdu.as_slice());
        prop_assert!(source.is_empty());
    }
}

#[test]
fn pdu_empty_truncated_oversized_and_unknown_vectors_are_rejected() {
    let empty = DecodeError::Truncated {
        expected: 1,
        actual: 0,
    };
    assert_eq!(decode_pdu_ref(&[]).unwrap_err(), empty);
    assert_eq!(decode_request(&[]).unwrap_err(), empty);
    assert_eq!(decode_response(&[]).unwrap_err(), empty);

    let raw_unknown = decode_pdu_ref(&[0]).expect("raw PDU views preserve unknown function codes");
    assert_eq!(raw_unknown.function_code, 0);
    assert!(raw_unknown.data.is_empty());

    assert_eq!(
        decode_request(&[FunctionCode::ReadHoldingRegisters.code(), 0, 1]).unwrap_err(),
        DecodeError::Truncated {
            expected: 4,
            actual: 2,
        }
    );
    assert_eq!(
        decode_request(&[0]).unwrap_err(),
        DecodeError::UnknownFunctionCode(0)
    );
    assert_eq!(
        decode_response(&[0]).unwrap_err(),
        DecodeError::UnknownFunctionCode(0)
    );

    let oversized = vec![0x41; MAX_PDU_SIZE + 1];
    let expected = DecodeError::PduTooLarge {
        length: MAX_PDU_SIZE + 1,
        maximum: MAX_PDU_SIZE,
    };
    assert_eq!(decode_pdu_ref(&oversized).unwrap_err(), expected);
    assert_eq!(decode_request(&oversized).unwrap_err(), expected);
    assert_eq!(decode_response(&oversized).unwrap_err(), expected);
}

#[test]
fn fixed_length_excess_and_declared_byte_count_vectors_are_rejected() {
    assert_eq!(
        decode_request(&[FunctionCode::ReadHoldingRegisters.code(), 0, 1, 0, 1, 0]).unwrap_err(),
        DecodeError::LengthMismatch {
            expected: 4,
            actual: 5,
        }
    );
    assert_eq!(
        decode_response(&[FunctionCode::WriteSingleRegister.code(), 0, 1, 0]).unwrap_err(),
        DecodeError::Truncated {
            expected: 4,
            actual: 3,
        }
    );
    assert_eq!(
        decode_response(&[FunctionCode::WriteSingleRegister.code(), 0, 1, 0, 2, 0]).unwrap_err(),
        DecodeError::LengthMismatch {
            expected: 4,
            actual: 5,
        }
    );
    assert_eq!(
        decode_response(&[FunctionCode::ReadHoldingRegisters.code(), 4, 0, 1]).unwrap_err(),
        DecodeError::ByteCountMismatch {
            declared: 4,
            actual: 2,
        }
    );
    assert!(matches!(
        decode_request(&[0x0F, 0, 0, 0, 8, 2, 0xFF]),
        Err(DecodeError::ByteCountMismatch { .. })
    ));
    assert!(matches!(
        decode_request(&[0x10, 0, 0, 0, 2, 6, 0, 1, 0, 2, 0, 3]),
        Err(DecodeError::ByteCountMismatch { .. })
    ));
    match decode_response(&[0x83, 0x02]).expect("defined exception response decodes") {
        ResponsePdu::Exception(exception) => {
            assert_eq!(exception.function_code, FunctionCode::ReadHoldingRegisters);
            assert_eq!(exception.exception_code.code(), 0x02);
        }
        other => panic!("unexpected exception dispatch: {other:?}"),
    }
    assert_eq!(
        decode_response(&[0x83]).unwrap_err(),
        DecodeError::Truncated {
            expected: 1,
            actual: 0,
        }
    );
    assert_eq!(
        decode_response(&[0x83, 0x02, 0]).unwrap_err(),
        DecodeError::LengthMismatch {
            expected: 1,
            actual: 2,
        }
    );
}

#[test]
fn nested_file_record_and_device_id_length_vectors_are_rejected() {
    let malformed_read_file = [0x14, 8, 0x06, 0, 1, 0, 0, 0, 1, 0];
    assert_eq!(
        decode_request(&malformed_read_file).unwrap_err(),
        DecodeError::InvalidFileRecordLength { length: 8 }
    );

    let invalid_reference_type = [0x14, 7, 0x05, 0, 1, 0, 0, 0, 1];
    assert_eq!(
        decode_request(&invalid_reference_type).unwrap_err(),
        DecodeError::InvalidReferenceType(0x05)
    );

    let truncated_write_file = [0x15, 9, 0x06, 0, 1, 0, 0, 0, 2, 0x12, 0x34];
    assert_eq!(
        decode_request(&truncated_write_file).unwrap_err(),
        DecodeError::ByteCountMismatch {
            declared: 11,
            actual: 9,
        }
    );

    let truncated_device_id = [
        0x2B, 0x0E, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x05, b'h', b'i',
    ];
    assert_eq!(
        decode_response(&truncated_device_id).unwrap_err(),
        DecodeError::Truncated {
            expected: 13,
            actual: 10,
        }
    );
}

#[test]
fn mbap_protocol_length_and_truncation_vectors_are_bounded() {
    let mut invalid_protocol = build_mbap_adu(1, 1, &[0x03]);
    invalid_protocol[3] = 1;
    let mut source = BytesMut::from(invalid_protocol.as_slice());
    assert!(matches!(
        MbapCodec.decode(&mut source),
        Err(FrameError::InvalidProtocolId(1))
    ));

    let mut invalid_length = build_mbap_adu(1, 1, &[0x03]);
    invalid_length[4] = 0;
    invalid_length[5] = 1;
    let mut source = BytesMut::from(invalid_length.as_slice());
    assert!(matches!(
        MbapCodec.decode(&mut source),
        Err(FrameError::InvalidLength { .. })
    ));

    let mut oversized_length = build_mbap_adu(1, 1, &[0x03]);
    let declared = u16::try_from(MAX_PDU_SIZE + 2).expect("MBAP overflow vector fits u16");
    oversized_length[4..6].copy_from_slice(&declared.to_be_bytes());
    let mut source = BytesMut::from(oversized_length.as_slice());
    assert!(matches!(
        MbapCodec.decode(&mut source),
        Err(FrameError::LengthOverflow(value)) if value == declared
    ));

    let truncated = build_mbap_adu(1, 1, &[0x03, 0, 0, 0, 1]);
    let mut source = BytesMut::from(&truncated[..truncated.len() - 1]);
    assert!(MbapCodec.decode(&mut source).unwrap().is_none());
    assert!(source.len() < MAX_TCP_ADU_SIZE);
}

#[test]
fn rtu_short_oversized_and_bad_crc_vectors_are_bounded() {
    let mut short = BytesMut::from(&[1, 3, 0][..]);
    assert!(RtuCodec.decode(&mut short).unwrap().is_none());

    let oversized = build_rtu_adu(1, &vec![0x03; MAX_PDU_SIZE + 1]);
    let mut source = BytesMut::from(oversized.as_slice());
    assert!(matches!(
        RtuCodec.decode(&mut source),
        Err(FrameError::PduLengthOverflow {
            length,
            maximum: MAX_PDU_SIZE,
        }) if length == MAX_PDU_SIZE + 1
    ));

    let mut bad_crc = build_rtu_adu(1, &[0x03, 0, 0, 0, 1]);
    let last = bad_crc.len() - 1;
    bad_crc[last] ^= 0x80;
    let mut source = BytesMut::from(bad_crc.as_slice());
    assert!(matches!(
        RtuCodec.decode(&mut source),
        Err(FrameError::CrcMismatch { .. })
    ));
}

#[test]
fn rtu_over_tcp_first_valid_crc_prefix_contract_is_preserved() {
    let mut ambiguous = build_rtu_adu(1, &[0x03]);
    ambiguous.push(0xA5);
    ambiguous.extend_from_slice(&crc16(&ambiguous).to_le_bytes());
    assert!(verify_crc(&ambiguous[..4]));
    assert!(verify_crc(&ambiguous));

    let mut source = BytesMut::from(ambiguous.as_slice());
    let frame = RtuOverTcpCodec
        .decode(&mut source)
        .unwrap()
        .expect("the first CRC-valid prefix is emitted");
    assert_eq!(frame.unit_id(), 1);
    assert_eq!(frame.pdu.as_ref(), &[0x03]);
    assert_eq!(source.as_ref(), &ambiguous[4..]);
}

#[test]
fn rtu_over_tcp_exact_maximum_crc_miss_is_terminal() {
    let exact_maximum = crc_miss_buffer(MAX_RTU_ADU_SIZE);
    let mut source = BytesMut::from(exact_maximum.as_slice());
    assert!(matches!(
        RtuOverTcpCodec.decode(&mut source),
        Err(FrameError::Truncated)
    ));
    assert_eq!(source.len(), MAX_RTU_ADU_SIZE);
}

#[test]
fn maximum_valid_mbap_and_rtu_adus_round_trip() {
    let maximum_pdu = vec![0x41; MAX_PDU_SIZE];

    let mbap = build_mbap_adu(u16::MAX, u8::MAX, &maximum_pdu);
    assert_eq!(mbap.len(), MAX_TCP_ADU_SIZE);
    let (frames, pending) = decode_with_schedule(
        MbapCodec,
        &mbap,
        &[1, MBAP_HEADER_LEN - 1, MAX_TCP_ADU_SIZE],
        MAX_TCP_ADU_SIZE,
    )
    .unwrap();
    assert_eq!(pending, 0);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].pdu, maximum_pdu);

    let rtu = build_unambiguous_rtu_adu(247, &maximum_pdu);
    assert_eq!(rtu.len(), MAX_RTU_ADU_SIZE);
    let mut physical_source = BytesMut::from(rtu.as_slice());
    let physical = RtuCodec.decode(&mut physical_source).unwrap().unwrap();
    assert_eq!(physical.unit_id(), 247);
    assert_eq!(physical.pdu.as_ref(), maximum_pdu.as_slice());
    assert!(physical_source.is_empty());

    let (frames, pending) = decode_with_schedule(
        RtuOverTcpCodec,
        &rtu,
        &[1, 3, MAX_RTU_ADU_SIZE],
        MAX_RTU_ADU_SIZE,
    )
    .unwrap();
    assert_eq!(pending, 0);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].pdu, maximum_pdu);
}
