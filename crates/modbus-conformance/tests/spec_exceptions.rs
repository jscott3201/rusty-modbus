//! Spec V1.1b3 §7 — MODBUS Exception Responses conformance tests.

use modbus_codec::decode_response;
use modbus_codec::response::ExceptionResponse;
use modbus_codec::request::Encode;
use modbus_codec::DecodeError;
use modbus_types::{ExceptionCode, FunctionCode};

// Spec example p.47: client reads coil at address 1185 (0x04A1)
// Server returns exception code 02 (Illegal Data Address)
const SPEC_EXCEPTION_RESPONSE: &[u8] = &[0x81, 0x02];

#[test]
fn spec_7_exception_example_decode_via_dispatch() {
    match decode_response(SPEC_EXCEPTION_RESPONSE).unwrap() {
        modbus_codec::ResponsePdu::Exception(exc) => {
            assert_eq!(exc.function_code, FunctionCode::ReadCoils);
            assert_eq!(exc.exception_code, ExceptionCode::IllegalDataAddress);
        }
        other => panic!("expected Exception, got {other:?}"),
    }
}

#[test]
fn spec_7_exception_direct_decode() {
    // ExceptionResponse::decode takes fc_byte and data separately
    let exc = ExceptionResponse::decode(0x81, &[0x02]).unwrap();
    assert_eq!(exc.function_code, FunctionCode::ReadCoils);
    assert_eq!(exc.exception_code, ExceptionCode::IllegalDataAddress);
}

#[test]
fn spec_7_every_fc_exception_byte() {
    // Each FC's exception response has FC | 0x80
    let fcs: [(FunctionCode, u8); 19] = [
        (FunctionCode::ReadCoils, 0x81),
        (FunctionCode::ReadDiscreteInputs, 0x82),
        (FunctionCode::ReadHoldingRegisters, 0x83),
        (FunctionCode::ReadInputRegisters, 0x84),
        (FunctionCode::WriteSingleCoil, 0x85),
        (FunctionCode::WriteSingleRegister, 0x86),
        (FunctionCode::ReadExceptionStatus, 0x87),
        (FunctionCode::Diagnostics, 0x88),
        (FunctionCode::GetCommEventCounter, 0x8B),
        (FunctionCode::GetCommEventLog, 0x8C),
        (FunctionCode::WriteMultipleCoils, 0x8F),
        (FunctionCode::WriteMultipleRegisters, 0x90),
        (FunctionCode::ReportServerId, 0x91),
        (FunctionCode::ReadFileRecord, 0x94),
        (FunctionCode::WriteFileRecord, 0x95),
        (FunctionCode::MaskWriteRegister, 0x96),
        (FunctionCode::ReadWriteMultipleRegisters, 0x97),
        (FunctionCode::ReadFifoQueue, 0x98),
        (FunctionCode::EncapsulatedInterfaceTransport, 0xAB),
    ];
    for (fc, exc_byte) in fcs {
        assert_eq!(fc.exception_code(), exc_byte, "exception byte mismatch for {fc:?}");
        // Verify decode
        let exc = ExceptionResponse::decode(exc_byte, &[0x01]).unwrap();
        assert_eq!(exc.function_code, fc);
        assert_eq!(exc.exception_code, ExceptionCode::IllegalFunction);
    }
}

#[test]
fn spec_7_all_exception_codes_decode() {
    let codes: [(u8, ExceptionCode); 10] = [
        (0x01, ExceptionCode::IllegalFunction),
        (0x02, ExceptionCode::IllegalDataAddress),
        (0x03, ExceptionCode::IllegalDataValue),
        (0x04, ExceptionCode::ServerDeviceFailure),
        (0x05, ExceptionCode::Acknowledge),
        (0x06, ExceptionCode::ServerDeviceBusy),
        (0x07, ExceptionCode::NegativeAcknowledge),
        (0x08, ExceptionCode::MemoryParityError),
        (0x0A, ExceptionCode::GatewayPathUnavailable),
        (0x0B, ExceptionCode::GatewayTargetDeviceFailedToRespond),
    ];
    for (byte, expected) in codes {
        let exc = ExceptionResponse::decode(0x81, &[byte]).unwrap();
        assert_eq!(exc.exception_code, expected, "exception code {byte:#04X}");
    }
}

#[test]
fn spec_7_custom_exception_code() {
    // 0x07 is now NegativeAcknowledge (named variant)
    let exc = ExceptionResponse::decode(0x81, &[0x07]).unwrap();
    assert_eq!(exc.exception_code, ExceptionCode::NegativeAcknowledge);
    // 0x09 is Custom
    let exc = ExceptionResponse::decode(0x81, &[0x09]).unwrap();
    assert_eq!(exc.exception_code, ExceptionCode::Custom(0x09));
}

#[test]
fn spec_7_exception_encode_round_trip() {
    let exc = ExceptionResponse {
        function_code: FunctionCode::ReadCoils,
        exception_code: ExceptionCode::IllegalDataAddress,
    };
    let mut buf = [0u8; 2];
    let len = exc.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], SPEC_EXCEPTION_RESPONSE);
}

#[test]
fn spec_7_exception_truncated() {
    // Exception with no data after FC byte
    assert!(matches!(
        ExceptionResponse::decode(0x81, &[]),
        Err(DecodeError::Truncated { .. })
    ));
}
