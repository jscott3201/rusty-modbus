//! Spec V1.1b3 §4, §5.1, §7 — Type round-trips and enum completeness.

use rusty_modbus_types::*;

// ── FunctionCode (§5.1) ────────────────────────────────────────────

#[test]
fn function_code_all_19_variants_round_trip() {
    let variants: [(FunctionCode, u8); 19] = [
        (FunctionCode::ReadCoils, 0x01),
        (FunctionCode::ReadDiscreteInputs, 0x02),
        (FunctionCode::ReadHoldingRegisters, 0x03),
        (FunctionCode::ReadInputRegisters, 0x04),
        (FunctionCode::WriteSingleCoil, 0x05),
        (FunctionCode::WriteSingleRegister, 0x06),
        (FunctionCode::ReadExceptionStatus, 0x07),
        (FunctionCode::Diagnostics, 0x08),
        (FunctionCode::GetCommEventCounter, 0x0B),
        (FunctionCode::GetCommEventLog, 0x0C),
        (FunctionCode::WriteMultipleCoils, 0x0F),
        (FunctionCode::WriteMultipleRegisters, 0x10),
        (FunctionCode::ReportServerId, 0x11),
        (FunctionCode::ReadFileRecord, 0x14),
        (FunctionCode::WriteFileRecord, 0x15),
        (FunctionCode::MaskWriteRegister, 0x16),
        (FunctionCode::ReadWriteMultipleRegisters, 0x17),
        (FunctionCode::ReadFifoQueue, 0x18),
        (FunctionCode::EncapsulatedInterfaceTransport, 0x2B),
    ];
    for (fc, wire) in variants {
        assert_eq!(fc.code(), wire, "code() mismatch for {fc:?}");
        assert_eq!(FunctionCode::from_raw(wire), Some(fc), "from_raw({wire:#04X}) mismatch");
    }
}

#[test]
fn function_code_zero_is_custom() {
    // §4.1: "Function code '0' is not valid" — represented as Custom, rejected by server
    assert_eq!(FunctionCode::from_raw(0x00), Some(FunctionCode::Custom(0x00)));
}

#[test]
fn function_code_from_raw_rejects_exception_flagged() {
    // Bytes 0x80+ are exception responses, not valid function codes
    for byte in 0x80..=0xFF {
        assert_eq!(FunctionCode::from_raw(byte), None, "from_raw({byte:#04X}) should be None");
    }
}

#[test]
fn function_code_is_exception_response() {
    for byte in 0x00..=0x7F {
        assert!(!FunctionCode::is_exception_response(byte));
    }
    for byte in 0x80..=0xFF {
        assert!(FunctionCode::is_exception_response(byte));
    }
}

#[test]
fn function_code_exception_code_sets_high_bit() {
    assert_eq!(FunctionCode::ReadCoils.exception_code(), 0x81);
    assert_eq!(FunctionCode::ReadHoldingRegisters.exception_code(), 0x83);
    assert_eq!(FunctionCode::EncapsulatedInterfaceTransport.exception_code(), 0xAB);
}

#[test]
fn function_code_from_exception_raw_strips_flag() {
    assert_eq!(FunctionCode::from_exception_raw(0x81), FunctionCode::ReadCoils);
    assert_eq!(FunctionCode::from_exception_raw(0x83), FunctionCode::ReadHoldingRegisters);
    // Also works without the flag
    assert_eq!(FunctionCode::from_exception_raw(0x01), FunctionCode::ReadCoils);
    // Custom vendor codes
    assert_eq!(FunctionCode::from_exception_raw(0xC1), FunctionCode::Custom(0x41));
}

// ── ExceptionCode (§7) ─────────────────────────────────────────────

#[test]
fn exception_code_all_10_variants_round_trip() {
    let variants: [(ExceptionCode, u8); 10] = [
        (ExceptionCode::IllegalFunction, 0x01),
        (ExceptionCode::IllegalDataAddress, 0x02),
        (ExceptionCode::IllegalDataValue, 0x03),
        (ExceptionCode::ServerDeviceFailure, 0x04),
        (ExceptionCode::Acknowledge, 0x05),
        (ExceptionCode::ServerDeviceBusy, 0x06),
        (ExceptionCode::NegativeAcknowledge, 0x07),
        (ExceptionCode::MemoryParityError, 0x08),
        (ExceptionCode::GatewayPathUnavailable, 0x0A),
        (ExceptionCode::GatewayTargetDeviceFailedToRespond, 0x0B),
    ];
    for (ec, wire) in variants {
        assert_eq!(ec.code(), wire, "code() mismatch for {ec:?}");
        assert_eq!(ExceptionCode::from_raw(wire), ec, "from_raw({wire:#04X}) mismatch");
    }
}

#[test]
fn exception_code_gaps_return_custom() {
    // Unknown codes become Custom
    assert_eq!(ExceptionCode::from_raw(0x00), ExceptionCode::Custom(0x00));
    assert_eq!(ExceptionCode::from_raw(0x09), ExceptionCode::Custom(0x09));
    assert_eq!(ExceptionCode::from_raw(0x0C), ExceptionCode::Custom(0x0C));
}

// ── DiagnosticSubFunction (§6.8.1) ─────────────────────────────────

#[test]
fn diagnostic_sub_function_all_15_variants_round_trip() {
    let variants: [(DiagnosticSubFunction, u16); 15] = [
        (DiagnosticSubFunction::ReturnQueryData, 0x0000),
        (DiagnosticSubFunction::RestartCommunicationsOption, 0x0001),
        (DiagnosticSubFunction::ReturnDiagnosticRegister, 0x0002),
        (DiagnosticSubFunction::ChangeAsciiInputDelimiter, 0x0003),
        (DiagnosticSubFunction::ForceListenOnlyMode, 0x0004),
        (DiagnosticSubFunction::ClearCountersAndDiagRegister, 0x000A),
        (DiagnosticSubFunction::ReturnBusMessageCount, 0x000B),
        (DiagnosticSubFunction::ReturnBusCommunicationErrorCount, 0x000C),
        (DiagnosticSubFunction::ReturnBusExceptionErrorCount, 0x000D),
        (DiagnosticSubFunction::ReturnServerMessageCount, 0x000E),
        (DiagnosticSubFunction::ReturnServerNoResponseCount, 0x000F),
        (DiagnosticSubFunction::ReturnServerNakCount, 0x0010),
        (DiagnosticSubFunction::ReturnServerBusyCount, 0x0011),
        (DiagnosticSubFunction::ReturnBusCharacterOverrunCount, 0x0012),
        (DiagnosticSubFunction::ClearOverrunCounterAndFlag, 0x0014),
    ];
    for (sf, wire) in variants {
        assert_eq!(sf.code(), wire, "code() mismatch for {sf:?}");
        assert_eq!(DiagnosticSubFunction::from_raw(wire), Some(sf));
    }
}

// ── CoilValue (§6.5) ───────────────────────────────────────────────

#[test]
fn coil_value_wire_encoding() {
    // §6.5: "A value of FF 00 hex requests the output to be ON.
    //         A value of 00 00 requests it to be OFF.
    //         All other values are illegal."
    assert_eq!(CoilValue::from_wire(0xFF00), Some(CoilValue::On));
    assert_eq!(CoilValue::from_wire(0x0000), Some(CoilValue::Off));
    assert_eq!(CoilValue::from_wire(0x0001), None);
    assert_eq!(CoilValue::from_wire(0xFF01), None);
    assert_eq!(CoilValue::from_wire(0x00FF), None);
}

#[test]
fn coil_value_round_trip() {
    assert_eq!(CoilValue::from_wire(CoilValue::On.to_wire()), Some(CoilValue::On));
    assert_eq!(CoilValue::from_wire(CoilValue::Off.to_wire()), Some(CoilValue::Off));
}

#[test]
fn coil_value_bool_conversion() {
    assert_eq!(CoilValue::from_bool(true), CoilValue::On);
    assert_eq!(CoilValue::from_bool(false), CoilValue::Off);
    assert!(CoilValue::On.as_bool());
    assert!(!CoilValue::Off.as_bool());
}

// ── UnitId (§4) ────────────────────────────────────────────────────

#[test]
fn unit_id_broadcast() {
    assert!(UnitId(0x00).is_broadcast());
    assert!(!UnitId(0x01).is_broadcast());
}

#[test]
fn unit_id_tcp_device() {
    assert!(UnitId(0xFF).is_tcp_device());
    assert!(!UnitId(0x01).is_tcp_device());
}

#[test]
fn unit_id_valid_slave_range() {
    assert!(!UnitId(0).is_valid_slave());
    for id in 1..=247u8 {
        assert!(UnitId(id).is_valid_slave(), "UnitId({id}) should be valid slave");
    }
    assert!(!UnitId(248).is_valid_slave());
    assert!(!UnitId(255).is_valid_slave());
}

// ── MeiType (§6.19) ───────────────────────────────────────────────

#[test]
fn mei_type_round_trip() {
    assert_eq!(MeiType::from_raw(0x0D), Some(MeiType::CanOpenGeneralReference));
    assert_eq!(MeiType::from_raw(0x0E), Some(MeiType::ReadDeviceIdentification));
    assert_eq!(MeiType::CanOpenGeneralReference.code(), 0x0D);
    assert_eq!(MeiType::ReadDeviceIdentification.code(), 0x0E);
}

// ── DeviceIdCode (§6.21) ──────────────────────────────────────────

#[test]
fn device_id_code_round_trip() {
    let variants: [(DeviceIdCode, u8); 4] = [
        (DeviceIdCode::BasicStream, 0x01),
        (DeviceIdCode::RegularStream, 0x02),
        (DeviceIdCode::ExtendedStream, 0x03),
        (DeviceIdCode::Individual, 0x04),
    ];
    for (code, wire) in variants {
        assert_eq!(code.code(), wire);
        assert_eq!(DeviceIdCode::from_raw(wire), Some(code));
    }
}

// ── DeviceIdObject (§6.21) ─────────────────────────────────────────

#[test]
fn device_id_object_round_trip() {
    let variants: [(DeviceIdObject, u8); 7] = [
        (DeviceIdObject::VendorName, 0x00),
        (DeviceIdObject::ProductCode, 0x01),
        (DeviceIdObject::MajorMinorRevision, 0x02),
        (DeviceIdObject::VendorUrl, 0x03),
        (DeviceIdObject::ProductName, 0x04),
        (DeviceIdObject::ModelName, 0x05),
        (DeviceIdObject::UserApplicationName, 0x06),
    ];
    for (obj, wire) in variants {
        assert_eq!(obj.code(), wire);
        assert_eq!(DeviceIdObject::from_raw(wire), Some(obj));
    }
}
