//! Integration tests: round-trip encode/decode, wire-format, and error conditions.

use modbus_codec::request::{
    MaskWriteRegisterRequest, ReadCoilsRequest, ReadFifoQueueRequest,
    ReadHoldingRegistersRequest, ReadWriteMultipleRegistersRequest, WriteMultipleCoilsRequest,
    WriteMultipleRegistersRequest, WriteSingleCoilRequest, WriteSingleRegisterRequest,
};
use modbus_codec::response::{
    ExceptionResponse, ReadHoldingRegistersResponse, WriteMultipleCoilsResponse,
    WriteMultipleRegistersResponse, WriteSingleCoilResponse,
};
use modbus_codec::{decode_request, decode_response, DecodeError, Encode, RequestPdu, ResponsePdu};
use modbus_types::{Address, CoilValue, ExceptionCode, FunctionCode, Quantity};

// ---------------------------------------------------------------------------
// Helper: encode a request into a stack buffer, return the used slice length.
// ---------------------------------------------------------------------------
fn encode_to_buf(encodable: &impl Encode, buf: &mut [u8]) -> usize {
    let len = encodable.encoded_len();
    let written = encodable.encode_into(buf).expect("encode failed");
    assert_eq!(written, len, "encode_into return value must match encoded_len");
    written
}

// =========================================================================
// 1. Round-trip tests for request types
// =========================================================================

#[test]
fn round_trip_fc01_read_coils() {
    let req = ReadCoilsRequest {
        address: Address(0x006B),
        quantity: Quantity(0x0013),
    };
    let mut buf = [0u8; 256];
    let len = encode_to_buf(&req, &mut buf);

    let decoded = decode_request(&buf[..len]).expect("decode failed");
    match decoded {
        RequestPdu::ReadCoils(r) => {
            assert_eq!(r.address, Address(0x006B));
            assert_eq!(r.quantity, Quantity(0x0013));
        }
        other => panic!("expected ReadCoils, got {other:?}"),
    }
}

#[test]
fn round_trip_fc03_read_holding_registers() {
    let req = ReadHoldingRegistersRequest {
        address: Address(0x006B),
        quantity: Quantity(3),
    };
    let mut buf = [0u8; 256];
    let len = encode_to_buf(&req, &mut buf);

    let decoded = decode_request(&buf[..len]).expect("decode failed");
    match decoded {
        RequestPdu::ReadHoldingRegisters(r) => {
            assert_eq!(r.address, Address(0x006B));
            assert_eq!(r.quantity, Quantity(3));
        }
        other => panic!("expected ReadHoldingRegisters, got {other:?}"),
    }
}

#[test]
fn round_trip_fc05_write_single_coil() {
    let req = WriteSingleCoilRequest {
        address: Address(0x00AC),
        value: CoilValue::On,
    };
    let mut buf = [0u8; 256];
    let len = encode_to_buf(&req, &mut buf);

    let decoded = decode_request(&buf[..len]).expect("decode failed");
    match decoded {
        RequestPdu::WriteSingleCoil(r) => {
            assert_eq!(r.address, Address(0x00AC));
            assert_eq!(r.value, CoilValue::On);
        }
        other => panic!("expected WriteSingleCoil, got {other:?}"),
    }
}

#[test]
fn round_trip_fc06_write_single_register() {
    let req = WriteSingleRegisterRequest {
        address: Address(0x0001),
        value: 0x0003,
    };
    let mut buf = [0u8; 256];
    let len = encode_to_buf(&req, &mut buf);

    let decoded = decode_request(&buf[..len]).expect("decode failed");
    match decoded {
        RequestPdu::WriteSingleRegister(r) => {
            assert_eq!(r.address, Address(0x0001));
            assert_eq!(r.value, 0x0003);
        }
        other => panic!("expected WriteSingleRegister, got {other:?}"),
    }
}

#[test]
fn round_trip_fc0f_write_multiple_coils() {
    let coil_values: &[u8] = &[0xCD, 0x01];
    let req = WriteMultipleCoilsRequest {
        address: Address(0x0013),
        quantity: Quantity(10),
        byte_count: 2,
        coil_values,
    };
    let mut buf = [0u8; 256];
    let len = encode_to_buf(&req, &mut buf);

    let decoded = decode_request(&buf[..len]).expect("decode failed");
    match decoded {
        RequestPdu::WriteMultipleCoils(r) => {
            assert_eq!(r.address, Address(0x0013));
            assert_eq!(r.quantity, Quantity(10));
            assert_eq!(r.byte_count, 2);
            assert_eq!(r.coil_values, &[0xCD, 0x01]);
        }
        other => panic!("expected WriteMultipleCoils, got {other:?}"),
    }
}

#[test]
fn round_trip_fc10_write_multiple_registers() {
    let register_values: &[u8] = &[0x00, 0x0A, 0x01, 0x02];
    let req = WriteMultipleRegistersRequest {
        address: Address(0x0001),
        quantity: Quantity(2),
        byte_count: 4,
        register_values,
    };
    let mut buf = [0u8; 256];
    let len = encode_to_buf(&req, &mut buf);

    let decoded = decode_request(&buf[..len]).expect("decode failed");
    match decoded {
        RequestPdu::WriteMultipleRegisters(r) => {
            assert_eq!(r.address, Address(0x0001));
            assert_eq!(r.quantity, Quantity(2));
            assert_eq!(r.byte_count, 4);
            assert_eq!(r.register_values, &[0x00, 0x0A, 0x01, 0x02]);
        }
        other => panic!("expected WriteMultipleRegisters, got {other:?}"),
    }
}

#[test]
fn round_trip_fc16_mask_write_register() {
    let req = MaskWriteRegisterRequest {
        address: Address(0x0004),
        and_mask: 0x00F2,
        or_mask: 0x0025,
    };
    let mut buf = [0u8; 256];
    let len = encode_to_buf(&req, &mut buf);

    let decoded = decode_request(&buf[..len]).expect("decode failed");
    match decoded {
        RequestPdu::MaskWriteRegister(r) => {
            assert_eq!(r.address, Address(0x0004));
            assert_eq!(r.and_mask, 0x00F2);
            assert_eq!(r.or_mask, 0x0025);
        }
        other => panic!("expected MaskWriteRegister, got {other:?}"),
    }
}

#[test]
fn round_trip_fc17_read_write_multiple_registers() {
    let write_values: &[u8] = &[0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF];
    let req = ReadWriteMultipleRegistersRequest {
        read_address: Address(0x0003),
        read_quantity: Quantity(6),
        write_address: Address(0x000E),
        write_quantity: Quantity(3),
        write_byte_count: 6,
        write_register_values: write_values,
    };
    let mut buf = [0u8; 256];
    let len = encode_to_buf(&req, &mut buf);

    let decoded = decode_request(&buf[..len]).expect("decode failed");
    match decoded {
        RequestPdu::ReadWriteMultipleRegisters(r) => {
            assert_eq!(r.read_address, Address(0x0003));
            assert_eq!(r.read_quantity, Quantity(6));
            assert_eq!(r.write_address, Address(0x000E));
            assert_eq!(r.write_quantity, Quantity(3));
            assert_eq!(r.write_byte_count, 6);
            assert_eq!(r.write_register_values, &[0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF]);
        }
        other => panic!("expected ReadWriteMultipleRegisters, got {other:?}"),
    }
}

#[test]
fn round_trip_fc18_read_fifo_queue() {
    let req = ReadFifoQueueRequest {
        fifo_pointer_address: Address(0x04DE),
    };
    let mut buf = [0u8; 256];
    let len = encode_to_buf(&req, &mut buf);

    let decoded = decode_request(&buf[..len]).expect("decode failed");
    match decoded {
        RequestPdu::ReadFifoQueue(r) => {
            assert_eq!(r.fifo_pointer_address, Address(0x04DE));
        }
        other => panic!("expected ReadFifoQueue, got {other:?}"),
    }
}

// =========================================================================
// 2. Round-trip tests for response types
// =========================================================================

#[test]
fn round_trip_response_fc03_read_holding_registers() {
    let register_data: &[u8] = &[0x02, 0x2B, 0x00, 0x00, 0x00, 0x64];
    let resp = ReadHoldingRegistersResponse {
        byte_count: 6,
        register_data,
    };
    let mut buf = [0u8; 256];
    let len = encode_to_buf(&resp, &mut buf);

    let decoded = decode_response(&buf[..len]).expect("decode failed");
    match decoded {
        ResponsePdu::ReadHoldingRegisters(r) => {
            assert_eq!(r.byte_count, 6);
            assert_eq!(r.register_data, &[0x02, 0x2B, 0x00, 0x00, 0x00, 0x64]);
            assert_eq!(r.register(0), 0x022B);
            assert_eq!(r.count(), 3);
        }
        other => panic!("expected ReadHoldingRegisters, got {other:?}"),
    }
}

#[test]
fn round_trip_response_fc05_write_single_coil() {
    let resp = WriteSingleCoilResponse {
        address: Address(0x00AC),
        value: CoilValue::On,
    };
    let mut buf = [0u8; 256];
    let len = encode_to_buf(&resp, &mut buf);

    let decoded = decode_response(&buf[..len]).expect("decode failed");
    match decoded {
        ResponsePdu::WriteSingleCoil(r) => {
            assert_eq!(r.address, Address(0x00AC));
            assert_eq!(r.value, CoilValue::On);
        }
        other => panic!("expected WriteSingleCoil, got {other:?}"),
    }
}

#[test]
fn round_trip_response_fc0f_write_multiple_coils() {
    let resp = WriteMultipleCoilsResponse {
        address: Address(0x0013),
        quantity: Quantity(10),
    };
    let mut buf = [0u8; 256];
    let len = encode_to_buf(&resp, &mut buf);

    let decoded = decode_response(&buf[..len]).expect("decode failed");
    match decoded {
        ResponsePdu::WriteMultipleCoils(r) => {
            assert_eq!(r.address, Address(0x0013));
            assert_eq!(r.quantity, Quantity(10));
        }
        other => panic!("expected WriteMultipleCoils, got {other:?}"),
    }
}

#[test]
fn round_trip_response_fc10_write_multiple_registers() {
    let resp = WriteMultipleRegistersResponse {
        address: Address(0x0001),
        quantity: Quantity(2),
    };
    let mut buf = [0u8; 256];
    let len = encode_to_buf(&resp, &mut buf);

    let decoded = decode_response(&buf[..len]).expect("decode failed");
    match decoded {
        ResponsePdu::WriteMultipleRegisters(r) => {
            assert_eq!(r.address, Address(0x0001));
            assert_eq!(r.quantity, Quantity(2));
        }
        other => panic!("expected WriteMultipleRegisters, got {other:?}"),
    }
}

#[test]
fn round_trip_response_exception() {
    let resp = ExceptionResponse {
        function_code: FunctionCode::ReadCoils,
        exception_code: ExceptionCode::IllegalDataAddress,
    };
    let mut buf = [0u8; 256];
    let len = encode_to_buf(&resp, &mut buf);

    // Verify the first byte has the exception flag set: 0x81.
    assert_eq!(buf[0], 0x81);

    let decoded = decode_response(&buf[..len]).expect("decode failed");
    match decoded {
        ResponsePdu::Exception(e) => {
            assert_eq!(e.function_code, FunctionCode::ReadCoils);
            assert_eq!(e.exception_code, ExceptionCode::IllegalDataAddress);
        }
        other => panic!("expected Exception, got {other:?}"),
    }
}

// =========================================================================
// 3. Wire-format byte-exact tests (Modbus spec examples)
// =========================================================================

#[test]
fn wire_format_fc03_request() {
    let req = ReadHoldingRegistersRequest {
        address: Address(0x006B),
        quantity: Quantity(3),
    };
    let mut buf = [0u8; 16];
    let len = encode_to_buf(&req, &mut buf);

    assert_eq!(&buf[..len], &[0x03, 0x00, 0x6B, 0x00, 0x03]);
}

#[test]
fn wire_format_fc03_response() {
    let register_data: &[u8] = &[0x02, 0x2B, 0x00, 0x00, 0x00, 0x64];
    let resp = ReadHoldingRegistersResponse {
        byte_count: 6,
        register_data,
    };
    let mut buf = [0u8; 16];
    let len = encode_to_buf(&resp, &mut buf);

    assert_eq!(
        &buf[..len],
        &[0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64]
    );
}

#[test]
fn wire_format_fc03_request_decode_from_bytes() {
    let wire: &[u8] = &[0x03, 0x00, 0x6B, 0x00, 0x03];
    let decoded = decode_request(wire).expect("decode failed");
    match decoded {
        RequestPdu::ReadHoldingRegisters(r) => {
            assert_eq!(r.address, Address(0x006B));
            assert_eq!(r.quantity, Quantity(3));
        }
        other => panic!("expected ReadHoldingRegisters, got {other:?}"),
    }
}

#[test]
fn wire_format_fc03_response_decode_from_bytes() {
    let wire: &[u8] = &[0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64];
    let decoded = decode_response(wire).expect("decode failed");
    match decoded {
        ResponsePdu::ReadHoldingRegisters(r) => {
            assert_eq!(r.register(0), 0x022B);
            assert_eq!(r.register(1), 0x0000);
            assert_eq!(r.register(2), 0x0064);
            assert_eq!(r.count(), 3);
        }
        other => panic!("expected ReadHoldingRegisters, got {other:?}"),
    }
}

#[test]
fn wire_format_exception_response() {
    // FC 0x83 = ReadHoldingRegisters exception, code 0x02 = IllegalDataAddress.
    let wire: &[u8] = &[0x83, 0x02];
    let decoded = decode_response(wire).expect("decode failed");
    match decoded {
        ResponsePdu::Exception(e) => {
            assert_eq!(e.function_code, FunctionCode::ReadHoldingRegisters);
            assert_eq!(e.exception_code, ExceptionCode::IllegalDataAddress);
        }
        other => panic!("expected Exception, got {other:?}"),
    }
}

// =========================================================================
// 4. Error condition tests
// =========================================================================

#[test]
fn error_empty_pdu_truncated() {
    let result = decode_request(&[]);
    assert_eq!(
        result.unwrap_err(),
        DecodeError::Truncated {
            expected: 1,
            actual: 0,
        }
    );
}

#[test]
fn error_empty_pdu_response_truncated() {
    let result = decode_response(&[]);
    assert_eq!(
        result.unwrap_err(),
        DecodeError::Truncated {
            expected: 1,
            actual: 0,
        }
    );
}

#[test]
fn error_unknown_function_code_becomes_custom() {
    // FC 0x00 is now decoded as Custom(0x00) with empty data
    let wire: &[u8] = &[0x00];
    let result = decode_request(wire).unwrap();
    assert!(matches!(result, modbus_codec::RequestPdu::Custom(0x00, &[])));
}

#[test]
fn error_fc03_request_truncated() {
    // FC 03 request needs FC(1) + address(2) + quantity(2) = 5 bytes.
    // Provide only 3 bytes (FC + 2 data bytes instead of 4).
    let wire: &[u8] = &[0x03, 0x00, 0x6B];
    let result = decode_request(wire);
    assert_eq!(
        result.unwrap_err(),
        DecodeError::Truncated {
            expected: 4,
            actual: 2,
        }
    );
}

#[test]
fn error_fc01_response_bad_byte_count() {
    // FC 01 response: byte_count=3 but only 2 data bytes follow.
    let wire: &[u8] = &[0x01, 0x03, 0xCD, 0x6B];
    let result = decode_response(wire);
    assert_eq!(
        result.unwrap_err(),
        DecodeError::ByteCountMismatch {
            declared: 3,
            actual: 2,
        }
    );
}

// =========================================================================
// 5. Property: encoded_len matches actual encode output
// =========================================================================

#[test]
fn encoded_len_matches_actual_output_requests() {
    let coil_values: &[u8] = &[0xCD, 0x01];
    let reg_values: &[u8] = &[0x00, 0x0A, 0x01, 0x02];
    let rw_values: &[u8] = &[0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF];

    let requests: Vec<Box<dyn Encode>> = vec![
        Box::new(ReadCoilsRequest {
            address: Address(0x006B),
            quantity: Quantity(19),
        }),
        Box::new(ReadHoldingRegistersRequest {
            address: Address(0x006B),
            quantity: Quantity(3),
        }),
        Box::new(WriteSingleCoilRequest {
            address: Address(0x00AC),
            value: CoilValue::On,
        }),
        Box::new(WriteSingleRegisterRequest {
            address: Address(0x0001),
            value: 0x0003,
        }),
        Box::new(MaskWriteRegisterRequest {
            address: Address(0x0004),
            and_mask: 0x00F2,
            or_mask: 0x0025,
        }),
        Box::new(ReadFifoQueueRequest {
            fifo_pointer_address: Address(0x04DE),
        }),
    ];

    for encodable in &requests {
        let mut buf = [0u8; 256];
        let written = encodable.encode_into(&mut buf).expect("encode failed");
        assert_eq!(
            written,
            encodable.encoded_len(),
            "encoded_len mismatch for encoded PDU starting with FC {:#04X}",
            buf[0]
        );
    }

    // Borrowing types tested separately (lifetime prevents Box<dyn Encode>).
    {
        let req = WriteMultipleCoilsRequest {
            address: Address(0x0013),
            quantity: Quantity(10),
            byte_count: 2,
            coil_values,
        };
        let mut buf = [0u8; 256];
        let written = req.encode_into(&mut buf).expect("encode failed");
        assert_eq!(written, req.encoded_len());
    }
    {
        let req = WriteMultipleRegistersRequest {
            address: Address(0x0001),
            quantity: Quantity(2),
            byte_count: 4,
            register_values: reg_values,
        };
        let mut buf = [0u8; 256];
        let written = req.encode_into(&mut buf).expect("encode failed");
        assert_eq!(written, req.encoded_len());
    }
    {
        let req = ReadWriteMultipleRegistersRequest {
            read_address: Address(0x0003),
            read_quantity: Quantity(6),
            write_address: Address(0x000E),
            write_quantity: Quantity(3),
            write_byte_count: 6,
            write_register_values: rw_values,
        };
        let mut buf = [0u8; 256];
        let written = req.encode_into(&mut buf).expect("encode failed");
        assert_eq!(written, req.encoded_len());
    }
}

#[test]
fn encoded_len_matches_actual_output_responses() {
    let reg_data: &[u8] = &[0x02, 0x2B, 0x00, 0x00, 0x00, 0x64];

    // Owned response types.
    let responses: Vec<Box<dyn Encode>> = vec![
        Box::new(WriteSingleCoilResponse {
            address: Address(0x00AC),
            value: CoilValue::On,
        }),
        Box::new(WriteMultipleCoilsResponse {
            address: Address(0x0013),
            quantity: Quantity(10),
        }),
        Box::new(WriteMultipleRegistersResponse {
            address: Address(0x0001),
            quantity: Quantity(2),
        }),
        Box::new(ExceptionResponse {
            function_code: FunctionCode::ReadCoils,
            exception_code: ExceptionCode::IllegalDataAddress,
        }),
    ];

    for encodable in &responses {
        let mut buf = [0u8; 256];
        let written = encodable.encode_into(&mut buf).expect("encode failed");
        assert_eq!(
            written,
            encodable.encoded_len(),
            "encoded_len mismatch for response PDU starting with FC {:#04X}",
            buf[0]
        );
    }

    // Borrowing response type.
    {
        let resp = ReadHoldingRegistersResponse {
            byte_count: 6,
            register_data: reg_data,
        };
        let mut buf = [0u8; 256];
        let written = resp.encode_into(&mut buf).expect("encode failed");
        assert_eq!(written, resp.encoded_len());
    }
}
