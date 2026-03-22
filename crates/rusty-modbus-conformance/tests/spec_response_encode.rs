//! Response encode conformance — verify encoded responses match spec byte sequences.
//!
//! The decode tests verify we READ spec bytes correctly. These tests verify
//! we WRITE spec-correct bytes from constructed response structs.

use rusty_modbus_codec::request::Encode;
use rusty_modbus_codec::response::*;
use rusty_modbus_types::{Address, CoilValue, ExceptionCode, FunctionCode, Quantity};

#[test]
fn spec_6_1_fc01_response_encode() {
    let resp = ReadCoilsResponse {
        byte_count: 3,
        coil_status: &[0xCD, 0x6B, 0x05],
    };
    let mut buf = [0u8; 10];
    let len = resp.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], &[0x01, 0x03, 0xCD, 0x6B, 0x05]);
}

#[test]
fn spec_6_3_fc03_response_encode() {
    // Spec p.15: regs 108-110 = [555, 0, 100]
    let data = [0x02, 0x2B, 0x00, 0x00, 0x00, 0x64];
    let resp = ReadHoldingRegistersResponse {
        byte_count: 6,
        register_data: &data,
    };
    let mut buf = [0u8; 16];
    let len = resp.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], &[0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64]);
}

#[test]
fn spec_6_4_fc04_response_encode() {
    let data = [0x00, 0x0A];
    let resp = ReadInputRegistersResponse {
        byte_count: 2,
        register_data: &data,
    };
    let mut buf = [0u8; 8];
    let len = resp.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], &[0x04, 0x02, 0x00, 0x0A]);
}

#[test]
fn spec_6_5_fc05_response_encode() {
    let resp = WriteSingleCoilResponse {
        address: Address(0x00AC),
        value: CoilValue::On,
    };
    let mut buf = [0u8; 8];
    let len = resp.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], &[0x05, 0x00, 0xAC, 0xFF, 0x00]);
}

#[test]
fn spec_6_6_fc06_response_encode() {
    let resp = WriteSingleRegisterResponse {
        address: Address(0x0001),
        value: 0x0003,
    };
    let mut buf = [0u8; 8];
    let len = resp.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], &[0x06, 0x00, 0x01, 0x00, 0x03]);
}

#[test]
fn spec_6_11_fc0f_response_encode() {
    let resp = WriteMultipleCoilsResponse {
        address: Address(0x0013),
        quantity: Quantity(0x000A),
    };
    let mut buf = [0u8; 8];
    let len = resp.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], &[0x0F, 0x00, 0x13, 0x00, 0x0A]);
}

#[test]
fn spec_6_12_fc10_response_encode() {
    let resp = WriteMultipleRegistersResponse {
        address: Address(0x0001),
        quantity: Quantity(0x0002),
    };
    let mut buf = [0u8; 8];
    let len = resp.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], &[0x10, 0x00, 0x01, 0x00, 0x02]);
}

#[test]
fn spec_6_16_fc16_response_encode() {
    let resp = MaskWriteRegisterResponse {
        address: Address(0x0004),
        and_mask: 0x00F2,
        or_mask: 0x0025,
    };
    let mut buf = [0u8; 10];
    let len = resp.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], &[0x16, 0x00, 0x04, 0x00, 0xF2, 0x00, 0x25]);
}

#[test]
fn spec_7_exception_response_encode() {
    let resp = ExceptionResponse {
        function_code: FunctionCode::ReadCoils,
        exception_code: ExceptionCode::IllegalDataAddress,
    };
    let mut buf = [0u8; 4];
    let len = resp.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], &[0x81, 0x02]);
}

#[test]
fn spec_encoded_len_matches_actual() {
    // Verify encoded_len() returns the correct value for every response type
    let coil_resp = ReadCoilsResponse { byte_count: 3, coil_status: &[0xCD, 0x6B, 0x05] };
    assert_eq!(coil_resp.encoded_len(), 5); // FC + byte_count + 3 data

    let reg_data = [0x02, 0x2B, 0x00, 0x00, 0x00, 0x64];
    let reg_resp = ReadHoldingRegistersResponse { byte_count: 6, register_data: &reg_data };
    assert_eq!(reg_resp.encoded_len(), 8); // FC + byte_count + 6 data

    let exc = ExceptionResponse {
        function_code: FunctionCode::ReadCoils,
        exception_code: ExceptionCode::IllegalFunction,
    };
    assert_eq!(exc.encoded_len(), 2); // FC|0x80 + exception_code
}
