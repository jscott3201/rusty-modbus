//! Spec V1.1b3 fixed-length PDU conformance.
//!
//! Fixed-size function PDUs must not ignore trailing bytes after their defined
//! fields. Variable-size functions are covered by their byte-count validators.

use rusty_modbus_codec::request::{
    FileSubRequest, MaskWriteRegisterRequest, ReadCoilsRequest, ReadDeviceIdentificationRequest,
    ReadDiscreteInputsRequest, ReadFifoQueueRequest, ReadHoldingRegistersRequest,
    ReadInputRegistersRequest, WriteSingleCoilRequest, WriteSingleRegisterRequest,
};
use rusty_modbus_codec::response::{
    ExceptionResponse, GetCommEventCounterResponse, MaskWriteRegisterResponse,
    ReadExceptionStatusResponse, WriteMultipleCoilsResponse, WriteMultipleRegistersResponse,
    WriteSingleCoilResponse, WriteSingleRegisterResponse,
};
use rusty_modbus_codec::{DecodeError, decode_request, decode_response};

fn assert_length_mismatch<T>(result: Result<T, DecodeError>, expected: usize, actual: usize) {
    match result {
        Err(DecodeError::LengthMismatch {
            expected: got_expected,
            actual: got_actual,
        }) => {
            assert_eq!(got_expected, expected);
            assert_eq!(got_actual, actual);
        }
        Err(other) => panic!("expected LengthMismatch, got {other:?}"),
        Ok(_) => panic!("expected LengthMismatch, got Ok"),
    }
}

#[test]
fn fixed_request_decoders_reject_trailing_data() {
    assert_length_mismatch(ReadCoilsRequest::decode(&[0, 1, 0, 1, 0]), 4, 5);
    assert_length_mismatch(ReadDiscreteInputsRequest::decode(&[0, 1, 0, 1, 0]), 4, 5);
    assert_length_mismatch(ReadHoldingRegistersRequest::decode(&[0, 1, 0, 1, 0]), 4, 5);
    assert_length_mismatch(ReadInputRegistersRequest::decode(&[0, 1, 0, 1, 0]), 4, 5);
    assert_length_mismatch(WriteSingleCoilRequest::decode(&[0, 1, 0xFF, 0, 0]), 4, 5);
    assert_length_mismatch(WriteSingleRegisterRequest::decode(&[0, 1, 0, 2, 0]), 4, 5);
    assert_length_mismatch(
        MaskWriteRegisterRequest::decode(&[0, 1, 0xFF, 0, 0, 0x0F, 0]),
        6,
        7,
    );
    assert_length_mismatch(ReadFifoQueueRequest::decode(&[0, 1, 0]), 2, 3);
    assert_length_mismatch(FileSubRequest::decode(&[0x06, 0, 1, 0, 0, 0, 1, 0]), 7, 8);
    assert_length_mismatch(
        ReadDeviceIdentificationRequest::decode(&[0x0E, 0x01, 0x00, 0]),
        3,
        4,
    );
}

#[test]
fn empty_request_dispatch_rejects_trailing_data() {
    assert_length_mismatch(decode_request(&[0x07, 0]), 0, 1);
    assert_length_mismatch(decode_request(&[0x0B, 0]), 0, 1);
    assert_length_mismatch(decode_request(&[0x0C, 0]), 0, 1);
    assert_length_mismatch(decode_request(&[0x11, 0]), 0, 1);
}

#[test]
fn fixed_response_decoders_reject_trailing_data() {
    assert_length_mismatch(WriteSingleCoilResponse::decode(&[0, 1, 0xFF, 0, 0]), 4, 5);
    assert_length_mismatch(WriteMultipleCoilsResponse::decode(&[0, 1, 0, 1, 0]), 4, 5);
    assert_length_mismatch(WriteSingleRegisterResponse::decode(&[0, 1, 0, 2, 0]), 4, 5);
    assert_length_mismatch(
        WriteMultipleRegistersResponse::decode(&[0, 1, 0, 1, 0]),
        4,
        5,
    );
    assert_length_mismatch(
        MaskWriteRegisterResponse::decode(&[0, 1, 0xFF, 0, 0, 0x0F, 0]),
        6,
        7,
    );
    assert_length_mismatch(ReadExceptionStatusResponse::decode(&[0, 1]), 1, 2);
    assert_length_mismatch(GetCommEventCounterResponse::decode(&[0, 0, 0, 1, 0]), 4, 5);
    assert_length_mismatch(ExceptionResponse::decode(0x81, &[0x02, 0]), 1, 2);
}

#[test]
fn device_id_response_rejects_trailing_object_bytes() {
    let pdu = [
        0x2B, 0x0E, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01, b'A', 0xFF,
    ];

    assert_length_mismatch(decode_response(&pdu), 9, 10);
}
