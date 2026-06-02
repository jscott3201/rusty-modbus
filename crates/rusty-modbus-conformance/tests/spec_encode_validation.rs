//! Encode-side validation for public request/response structs.
//!
//! Decode tests prove malformed wire PDUs are rejected. These tests prove callers
//! also cannot serialize malformed public structs with inconsistent count fields.

use rusty_modbus_codec::request::*;
use rusty_modbus_codec::response::*;
use rusty_modbus_codec::{DecodeError, Encode, EncodeError, decode_request, decode_response};
use rusty_modbus_types::{Address, MAX_PDU_SIZE, Quantity};

fn encode_err(value: &impl Encode) -> EncodeError {
    let mut buf = [0u8; 300];
    value.encode_into(&mut buf).unwrap_err()
}

fn assert_quantity_out_of_range(err: EncodeError, quantity: u16) {
    assert_eq!(err, EncodeError::QuantityOutOfRange { quantity });
}

fn assert_byte_count_mismatch(err: EncodeError, declared: usize, actual: usize) {
    assert_eq!(err, EncodeError::ByteCountMismatch { declared, actual });
}

fn assert_byte_count_out_of_range(err: EncodeError, count: usize, minimum: usize, maximum: usize) {
    assert_eq!(
        err,
        EncodeError::ByteCountOutOfRange {
            count,
            minimum,
            maximum,
        }
    );
}

fn assert_pdu_too_large(err: EncodeError, length: usize) {
    assert_eq!(
        err,
        EncodeError::PduTooLarge {
            length,
            maximum: MAX_PDU_SIZE,
        }
    );
}

#[test]
fn decode_rejects_pdus_larger_than_253_bytes() {
    let request = [0x41; MAX_PDU_SIZE + 1];
    assert_eq!(
        decode_request(&request).unwrap_err(),
        DecodeError::PduTooLarge {
            length: MAX_PDU_SIZE + 1,
            maximum: MAX_PDU_SIZE,
        }
    );

    let response = [0x41; MAX_PDU_SIZE + 1];
    assert_eq!(
        decode_response(&response).unwrap_err(),
        DecodeError::PduTooLarge {
            length: MAX_PDU_SIZE + 1,
            maximum: MAX_PDU_SIZE,
        }
    );
}

#[test]
fn encode_rejects_read_request_quantities_out_of_range() {
    assert_quantity_out_of_range(
        encode_err(&ReadCoilsRequest {
            address: Address(0),
            quantity: Quantity(0),
        }),
        0,
    );
    assert_quantity_out_of_range(
        encode_err(&ReadDiscreteInputsRequest {
            address: Address(0),
            quantity: Quantity(2001),
        }),
        2001,
    );
    assert_quantity_out_of_range(
        encode_err(&ReadHoldingRegistersRequest {
            address: Address(0),
            quantity: Quantity(126),
        }),
        126,
    );
    assert_quantity_out_of_range(
        encode_err(&ReadInputRegistersRequest {
            address: Address(0),
            quantity: Quantity(0),
        }),
        0,
    );
}

#[test]
fn encode_rejects_fc0f_quantity_and_byte_count_mismatches() {
    assert_quantity_out_of_range(
        encode_err(&WriteMultipleCoilsRequest {
            address: Address(0),
            quantity: Quantity(1969),
            byte_count: 247,
            coil_values: &[0; 247],
        }),
        1969,
    );

    assert_byte_count_mismatch(
        encode_err(&WriteMultipleCoilsRequest {
            address: Address(0),
            quantity: Quantity(8),
            byte_count: 2,
            coil_values: &[0xFF, 0x00],
        }),
        2,
        1,
    );

    assert_byte_count_mismatch(
        encode_err(&WriteMultipleCoilsRequest {
            address: Address(0),
            quantity: Quantity(9),
            byte_count: 2,
            coil_values: &[0xFF],
        }),
        2,
        1,
    );
}

#[test]
fn encode_rejects_fc10_quantity_and_byte_count_mismatches() {
    assert_quantity_out_of_range(
        encode_err(&WriteMultipleRegistersRequest {
            address: Address(0),
            quantity: Quantity(124),
            byte_count: 248,
            register_values: &[0; 248],
        }),
        124,
    );

    assert_byte_count_mismatch(
        encode_err(&WriteMultipleRegistersRequest {
            address: Address(0),
            quantity: Quantity(2),
            byte_count: 6,
            register_values: &[0, 1, 2, 3, 4, 5],
        }),
        6,
        4,
    );

    assert_byte_count_mismatch(
        encode_err(&WriteMultipleRegistersRequest {
            address: Address(0),
            quantity: Quantity(2),
            byte_count: 4,
            register_values: &[0, 1],
        }),
        4,
        2,
    );
}

#[test]
fn encode_rejects_fc17_quantity_and_byte_count_mismatches() {
    assert_quantity_out_of_range(
        encode_err(&ReadWriteMultipleRegistersRequest {
            read_address: Address(0),
            read_quantity: Quantity(126),
            write_address: Address(0),
            write_quantity: Quantity(1),
            write_byte_count: 2,
            write_register_values: &[0, 1],
        }),
        126,
    );

    assert_quantity_out_of_range(
        encode_err(&ReadWriteMultipleRegistersRequest {
            read_address: Address(0),
            read_quantity: Quantity(1),
            write_address: Address(0),
            write_quantity: Quantity(122),
            write_byte_count: 244,
            write_register_values: &[0; 244],
        }),
        122,
    );

    assert_byte_count_mismatch(
        encode_err(&ReadWriteMultipleRegistersRequest {
            read_address: Address(0),
            read_quantity: Quantity(1),
            write_address: Address(0),
            write_quantity: Quantity(2),
            write_byte_count: 6,
            write_register_values: &[0, 1, 2, 3, 4, 5],
        }),
        6,
        4,
    );
}

#[test]
fn encode_rejects_file_record_request_byte_count_mismatches() {
    assert_byte_count_out_of_range(
        encode_err(&ReadFileRecordRequest {
            byte_count: 6,
            sub_requests: &[0; 6],
        }),
        6,
        7,
        245,
    );

    assert_byte_count_out_of_range(
        encode_err(&WriteFileRecordRequest {
            byte_count: 8,
            sub_requests: &[0; 8],
        }),
        8,
        9,
        251,
    );

    assert_byte_count_mismatch(
        encode_err(&ReadFileRecordRequest {
            byte_count: 7,
            sub_requests: &[0; 6],
        }),
        7,
        6,
    );

    assert_byte_count_mismatch(
        encode_err(&WriteFileRecordRequest {
            byte_count: 9,
            sub_requests: &[0; 10],
        }),
        9,
        10,
    );

    assert_eq!(
        encode_err(&ReadFileRecordRequest {
            byte_count: 7,
            sub_requests: &[0x07, 0, 1, 0, 0, 0, 1],
        }),
        EncodeError::InvalidReferenceType(0x07)
    );

    assert_eq!(
        encode_err(&WriteFileRecordRequest {
            byte_count: 9,
            sub_requests: &[0x07, 0, 1, 0, 0, 0, 1, 0x12, 0x34],
        }),
        EncodeError::InvalidReferenceType(0x07)
    );
}

#[test]
fn encode_rejects_response_byte_count_mismatches() {
    assert_byte_count_mismatch(
        encode_err(&ReadCoilsResponse {
            byte_count: 3,
            coil_status: &[0xCD, 0x6B],
        }),
        3,
        2,
    );
    assert_byte_count_mismatch(
        encode_err(&ReadDiscreteInputsResponse {
            byte_count: 1,
            coil_status: &[0xCD, 0x6B],
        }),
        1,
        2,
    );
    assert_byte_count_mismatch(
        encode_err(&ReadHoldingRegistersResponse {
            byte_count: 6,
            register_data: &[0, 1, 2, 3],
        }),
        6,
        4,
    );
    assert_byte_count_mismatch(
        encode_err(&ReadInputRegistersResponse {
            byte_count: 2,
            register_data: &[0, 1, 2, 3],
        }),
        2,
        4,
    );
    assert_byte_count_mismatch(
        encode_err(&ReadWriteMultipleRegistersResponse {
            byte_count: 6,
            register_data: &[0, 1, 2, 3],
        }),
        6,
        4,
    );
}

#[test]
fn encode_rejects_pdus_larger_than_253_bytes() {
    assert_pdu_too_large(
        encode_err(&ReadHoldingRegistersResponse {
            byte_count: 255,
            register_data: &[0; 255],
        }),
        257,
    );

    assert_pdu_too_large(
        encode_err(&ReadCoilsResponse {
            byte_count: 255,
            coil_status: &[0; 255],
        }),
        257,
    );
}

#[test]
fn encode_rejects_file_and_diagnostic_response_byte_count_mismatches() {
    assert_byte_count_out_of_range(
        encode_err(&ReadFileRecordResponse {
            byte_count: 3,
            data: &[0; 3],
        }),
        3,
        4,
        250,
    );

    assert_byte_count_out_of_range(
        encode_err(&WriteFileRecordResponse {
            byte_count: 252,
            data: &[0; 252],
        }),
        252,
        9,
        251,
    );

    assert_byte_count_mismatch(
        encode_err(&ReadFileRecordResponse {
            byte_count: 8,
            data: &[0; 7],
        }),
        8,
        7,
    );
    assert_byte_count_mismatch(
        encode_err(&WriteFileRecordResponse {
            byte_count: 9,
            data: &[0; 10],
        }),
        9,
        10,
    );

    assert_eq!(
        encode_err(&ReadFileRecordResponse {
            byte_count: 4,
            data: &[0x03, 0x07, 0x12, 0x34],
        }),
        EncodeError::InvalidReferenceType(0x07)
    );

    assert_eq!(
        encode_err(&WriteFileRecordResponse {
            byte_count: 9,
            data: &[0x07, 0, 1, 0, 0, 0, 1, 0x12, 0x34],
        }),
        EncodeError::InvalidReferenceType(0x07)
    );
    assert_byte_count_mismatch(
        encode_err(&GetCommEventLogResponse {
            byte_count: 7,
            status: 0,
            event_count: 0,
            message_count: 0,
            events: &[],
        }),
        7,
        6,
    );
    assert_byte_count_mismatch(
        encode_err(&ReportServerIdResponse {
            byte_count: 4,
            data: &[0x01, 0xFF],
        }),
        4,
        2,
    );
}

#[test]
fn encode_rejects_fifo_response_count_mismatches() {
    assert_byte_count_mismatch(
        encode_err(&ReadFifoQueueResponse {
            byte_count: 6,
            fifo_count: 1,
            fifo_values: &[0, 1],
        }),
        6,
        4,
    );

    assert_quantity_out_of_range(
        encode_err(&ReadFifoQueueResponse {
            byte_count: 66,
            fifo_count: 32,
            fifo_values: &[0; 64],
        }),
        32,
    );

    assert_byte_count_mismatch(
        encode_err(&ReadFifoQueueResponse {
            byte_count: 4,
            fifo_count: 2,
            fifo_values: &[0, 1],
        }),
        4,
        2,
    );
}

#[test]
fn encode_rejects_write_multiple_response_quantities_out_of_range() {
    assert_quantity_out_of_range(
        encode_err(&WriteMultipleCoilsResponse {
            address: Address(0),
            quantity: Quantity(1969),
        }),
        1969,
    );

    assert_quantity_out_of_range(
        encode_err(&WriteMultipleRegistersResponse {
            address: Address(0),
            quantity: Quantity(124),
        }),
        124,
    );
}
