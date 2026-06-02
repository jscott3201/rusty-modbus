//! Owned response type conformance tests.
//!
//! Verifies OwnedResponsePdu::from_pdu correctly dispatches spec example
//! PDUs to the right owned types with correct field values.

use bytes::Bytes;
use rusty_modbus_codec::DecodeError;
use rusty_modbus_frame::OwnedResponsePdu;
use rusty_modbus_types::{DiagnosticSubFunction, ExceptionCode, FunctionCode};

// ── FC 01 Read Coils ───────────────────────────────────────────────

#[test]
fn owned_fc01_read_coils() {
    let pdu = Bytes::from_static(&[0x01, 0x03, 0xCD, 0x6B, 0x05]);
    match OwnedResponsePdu::from_pdu(pdu).unwrap() {
        OwnedResponsePdu::ReadCoils(r) => {
            assert_eq!(r.byte_count, 3);
            assert!(r.coil(0)); // output 20 ON
            assert!(!r.coil(1)); // output 21 OFF
        }
        other => panic!("expected ReadCoils, got {other:?}"),
    }
}

// ── FC 02 Read Discrete Inputs ─────────────────────────────────────

#[test]
fn owned_fc02_read_discrete_inputs() {
    let pdu = Bytes::from_static(&[0x02, 0x03, 0xAC, 0xDB, 0x35]);
    match OwnedResponsePdu::from_pdu(pdu).unwrap() {
        OwnedResponsePdu::ReadDiscreteInputs(r) => {
            assert_eq!(r.byte_count, 3);
        }
        other => panic!("expected ReadDiscreteInputs, got {other:?}"),
    }
}

// ── FC 03 Read Holding Registers ───────────────────────────────────

#[test]
fn owned_fc03_read_holding_registers() {
    // Spec example: regs 108-110 = [555, 0, 100]
    let pdu = Bytes::from_static(&[0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64]);
    match OwnedResponsePdu::from_pdu(pdu).unwrap() {
        OwnedResponsePdu::ReadHoldingRegisters(r) => {
            assert_eq!(r.byte_count, 6);
            assert_eq!(r.count(), 3);
            assert_eq!(r.register(0), 555);
            assert_eq!(r.register(1), 0);
            assert_eq!(r.register(2), 100);
            let regs: Vec<u16> = r.registers().collect();
            assert_eq!(regs, vec![555, 0, 100]);
        }
        other => panic!("expected ReadHoldingRegisters, got {other:?}"),
    }
}

// ── FC 04 Read Input Registers ─────────────────────────────────────

#[test]
fn owned_fc04_read_input_registers() {
    let pdu = Bytes::from_static(&[0x04, 0x02, 0x00, 0x0A]);
    match OwnedResponsePdu::from_pdu(pdu).unwrap() {
        OwnedResponsePdu::ReadInputRegisters(r) => {
            assert_eq!(r.count(), 1);
            assert_eq!(r.register(0), 10);
        }
        other => panic!("expected ReadInputRegisters, got {other:?}"),
    }
}

// ── FC 05 Write Single Coil ───────────────────────────────────────

#[test]
fn owned_fc05_write_single_coil() {
    let pdu = Bytes::from_static(&[0x05, 0x00, 0xAC, 0xFF, 0x00]);
    match OwnedResponsePdu::from_pdu(pdu).unwrap() {
        OwnedResponsePdu::WriteSingleCoil(r) => {
            assert_eq!(r.address.0, 0x00AC);
            assert_eq!(r.value, rusty_modbus_types::CoilValue::On);
        }
        other => panic!("expected WriteSingleCoil, got {other:?}"),
    }
}

// ── FC 06 Write Single Register ───────────────────────────────────

#[test]
fn owned_fc06_write_single_register() {
    let pdu = Bytes::from_static(&[0x06, 0x00, 0x01, 0x00, 0x03]);
    match OwnedResponsePdu::from_pdu(pdu).unwrap() {
        OwnedResponsePdu::WriteSingleRegister(r) => {
            assert_eq!(r.address.0, 0x0001);
            assert_eq!(r.value, 0x0003);
        }
        other => panic!("expected WriteSingleRegister, got {other:?}"),
    }
}

// ── FC 07 Read Exception Status ───────────────────────────────────

#[test]
fn owned_fc07_read_exception_status() {
    let pdu = Bytes::from_static(&[0x07, 0x6D]);
    match OwnedResponsePdu::from_pdu(pdu).unwrap() {
        OwnedResponsePdu::ReadExceptionStatus(r) => {
            assert_eq!(r.status, 0x6D);
        }
        other => panic!("expected ReadExceptionStatus, got {other:?}"),
    }
}

// ── FC 08 Diagnostics ─────────────────────────────────────────────

#[test]
fn owned_fc08_diagnostics() {
    let pdu = Bytes::from_static(&[0x08, 0x00, 0x00, 0xA5, 0x37]);
    match OwnedResponsePdu::from_pdu(pdu).unwrap() {
        OwnedResponsePdu::Diagnostics(r) => {
            assert_eq!(r.sub_function, DiagnosticSubFunction::ReturnQueryData);
            assert_eq!(r.data, Bytes::from_static(&[0xA5, 0x37]));
        }
        other => panic!("expected Diagnostics, got {other:?}"),
    }
}

#[test]
fn owned_fc08_odd_data_length_errors() {
    let pdu = Bytes::from_static(&[0x08, 0x00, 0x00, 0xA5]);
    assert_eq!(
        OwnedResponsePdu::from_pdu(pdu).unwrap_err(),
        DecodeError::InvalidDiagnosticDataLength { length: 1 }
    );
}

#[test]
fn owned_fc08_unknown_subfunction_errors() {
    let pdu = Bytes::from_static(&[0x08, 0x00, 0x05, 0x00, 0x00]);
    assert_eq!(
        OwnedResponsePdu::from_pdu(pdu).unwrap_err(),
        DecodeError::UnknownDiagnosticSubFunction(0x0005)
    );
}

// ── FC 0B Get Comm Event Counter ──────────────────────────────────

#[test]
fn owned_fc0b_get_comm_event_counter() {
    let pdu = Bytes::from_static(&[0x0B, 0xFF, 0xFF, 0x01, 0x08]);
    match OwnedResponsePdu::from_pdu(pdu).unwrap() {
        OwnedResponsePdu::GetCommEventCounter(r) => {
            assert_eq!(r.status, 0xFFFF);
            assert_eq!(r.event_count, 0x0108);
        }
        other => panic!("expected GetCommEventCounter, got {other:?}"),
    }
}

// ── FC 0C Get Comm Event Log ─────────────────────────────────────

#[test]
fn owned_fc0c_get_comm_event_log() {
    let pdu = Bytes::from_static(&[0x0C, 0x08, 0x00, 0x00, 0x01, 0x08, 0x01, 0x21, 0x20, 0x00]);
    match OwnedResponsePdu::from_pdu(pdu).unwrap() {
        OwnedResponsePdu::GetCommEventLog(r) => {
            assert_eq!(r.byte_count, 0x08);
            assert_eq!(r.status, 0x0000);
            assert_eq!(r.event_count, 0x0108);
            assert_eq!(r.message_count, 0x0121);
            assert_eq!(r.events, Bytes::from_static(&[0x20, 0x00]));
        }
        other => panic!("expected GetCommEventLog, got {other:?}"),
    }
}

#[test]
fn owned_fc0c_rejects_byte_count_under_fixed_fields() {
    let pdu = Bytes::from_static(&[0x0C, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01]);
    assert_eq!(
        OwnedResponsePdu::from_pdu(pdu).unwrap_err(),
        DecodeError::ByteCountMismatch {
            declared: 0,
            actual: 6,
        }
    );
}

// ── FC 0F Write Multiple Coils ────────────────────────────────────

#[test]
fn owned_fc0f_write_multiple_coils() {
    let pdu = Bytes::from_static(&[0x0F, 0x00, 0x13, 0x00, 0x0A]);
    match OwnedResponsePdu::from_pdu(pdu).unwrap() {
        OwnedResponsePdu::WriteMultipleCoils(r) => {
            assert_eq!(r.address.0, 0x0013);
            assert_eq!(r.quantity.0, 0x000A);
        }
        other => panic!("expected WriteMultipleCoils, got {other:?}"),
    }
}

// ── FC 10 Write Multiple Registers ────────────────────────────────

#[test]
fn owned_fc10_write_multiple_registers() {
    let pdu = Bytes::from_static(&[0x10, 0x00, 0x01, 0x00, 0x02]);
    match OwnedResponsePdu::from_pdu(pdu).unwrap() {
        OwnedResponsePdu::WriteMultipleRegisters(r) => {
            assert_eq!(r.address.0, 0x0001);
            assert_eq!(r.quantity.0, 0x0002);
        }
        other => panic!("expected WriteMultipleRegisters, got {other:?}"),
    }
}

// ── FC 16 Mask Write Register ─────────────────────────────────────

#[test]
fn owned_fc16_mask_write_register() {
    let pdu = Bytes::from_static(&[0x16, 0x00, 0x04, 0x00, 0xF2, 0x00, 0x25]);
    match OwnedResponsePdu::from_pdu(pdu).unwrap() {
        OwnedResponsePdu::MaskWriteRegister(r) => {
            assert_eq!(r.address.0, 0x0004);
            assert_eq!(r.and_mask, 0x00F2);
            assert_eq!(r.or_mask, 0x0025);
        }
        other => panic!("expected MaskWriteRegister, got {other:?}"),
    }
}

// ── FC 18 Read FIFO Queue ─────────────────────────────────────────

#[test]
fn owned_fc18_read_fifo_queue() {
    let pdu = Bytes::from_static(&[0x18, 0x00, 0x06, 0x00, 0x02, 0x01, 0xB8, 0x12, 0x84]);
    match OwnedResponsePdu::from_pdu(pdu).unwrap() {
        OwnedResponsePdu::ReadFifoQueue(r) => {
            assert_eq!(r.byte_count, 0x0006);
            assert_eq!(r.fifo_count, 0x0002);
        }
        other => panic!("expected ReadFifoQueue, got {other:?}"),
    }
}

#[test]
fn owned_fc18_rejects_fifo_count_data_mismatch() {
    let pdu = Bytes::from_static(&[0x18, 0x00, 0x02, 0x00, 0x01]);
    assert_eq!(
        OwnedResponsePdu::from_pdu(pdu).unwrap_err(),
        DecodeError::ByteCountMismatch {
            declared: 2,
            actual: 0,
        }
    );
}

#[test]
fn owned_fc18_rejects_fifo_count_out_of_range() {
    let mut pdu = vec![0x18, 0x00, 0x42, 0x00, 0x20];
    pdu.extend([0; 64]);

    assert_eq!(
        OwnedResponsePdu::from_pdu(Bytes::from(pdu)).unwrap_err(),
        DecodeError::QuantityOutOfRange { quantity: 32 }
    );
}

// ── FC 14/15 File Record ─────────────────────────────────────────

#[test]
fn owned_fc14_rejects_invalid_reference_type() {
    let pdu = Bytes::from_static(&[0x14, 0x04, 0x03, 0x07, 0x12, 0x34]);
    assert_eq!(
        OwnedResponsePdu::from_pdu(pdu).unwrap_err(),
        DecodeError::InvalidReferenceType(0x07)
    );
}

#[test]
fn owned_fc15_rejects_invalid_reference_type() {
    let pdu = Bytes::from_static(&[0x15, 0x09, 0x07, 0, 1, 0, 0, 0, 1, 0x12, 0x34]);
    assert_eq!(
        OwnedResponsePdu::from_pdu(pdu).unwrap_err(),
        DecodeError::InvalidReferenceType(0x07)
    );
}

// ── FC 2B Encapsulated Interface ──────────────────────────────────

#[test]
fn owned_fc2b_encapsulated_interface() {
    let pdu = Bytes::from_static(&[0x2B, 0x0E, 0x01, 0x00, 0x00]);
    match OwnedResponsePdu::from_pdu(pdu).unwrap() {
        OwnedResponsePdu::EncapsulatedInterface(r) => {
            assert_eq!(
                r.mei_type,
                rusty_modbus_types::MeiType::ReadDeviceIdentification
            );
        }
        other => panic!("expected EncapsulatedInterface, got {other:?}"),
    }
}

// ── Exception Response ────────────────────────────────────────────

#[test]
fn owned_exception_response() {
    // FC01 exception: illegal data address
    let pdu = Bytes::from_static(&[0x81, 0x02]);
    match OwnedResponsePdu::from_pdu(pdu).unwrap() {
        OwnedResponsePdu::Exception(exc) => {
            assert_eq!(exc.function_code, FunctionCode::ReadCoils);
            assert_eq!(exc.exception_code, ExceptionCode::IllegalDataAddress);
        }
        other => panic!("expected Exception, got {other:?}"),
    }
}

// ── Error handling ────────────────────────────────────────────────

#[test]
fn owned_empty_pdu_errors() {
    let pdu = Bytes::from_static(&[]);
    assert!(OwnedResponsePdu::from_pdu(pdu).is_err());
}

#[test]
fn owned_unknown_fc_becomes_custom() {
    let pdu = Bytes::from_static(&[0x50, 0x00]);
    let result = OwnedResponsePdu::from_pdu(pdu).unwrap();
    assert!(matches!(result, OwnedResponsePdu::Custom(0x50, _)));
}

// ── FC 03 Response byte count mismatch ────────────────────────────

#[test]
fn owned_fc03_byte_count_mismatch() {
    // byte_count says 6 but only 4 data bytes follow
    let pdu = Bytes::from_static(&[0x03, 0x06, 0x02, 0x2B, 0x00, 0x00]);
    assert!(OwnedResponsePdu::from_pdu(pdu).is_err());
}
