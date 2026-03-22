//! Spec V1.1b3 §4.1, §5.1, §6.x — Protocol constant verification.
//!
//! Every assertion uses a literal value from the spec, not the constant itself,
//! to catch typos in the constant definition.

use rusty_modbus_types::*;

// ── PDU / ADU sizes (§4.1) ─────────────────────────────────────────

#[test]
fn spec_4_1_max_pdu_size() {
    // RS485 ADU = 256, minus address(1) minus CRC(2) = 253
    assert_eq!(MAX_PDU_SIZE, 253);
}

#[test]
fn spec_tcp_guide_3_1_3_mbap_header_len() {
    // TxnId(2) + ProtoId(2) + Len(2) + UnitId(1) = 7
    assert_eq!(MBAP_HEADER_LEN, 7);
}

#[test]
fn spec_4_1_max_tcp_adu_size() {
    // MBAP(7) + PDU(253) = 260
    assert_eq!(MAX_TCP_ADU_SIZE, 260);
}

#[test]
fn spec_4_1_max_rtu_adu_size() {
    // Address(1) + PDU(253) + CRC(2) = 256
    assert_eq!(MAX_RTU_ADU_SIZE, 256);
}

#[test]
fn spec_rtu_crc_len() {
    assert_eq!(RTU_CRC_LEN, 2);
}

// ── Protocol identifiers ──────────────────────────────────────────

#[test]
fn spec_tcp_guide_3_1_3_protocol_id() {
    assert_eq!(MODBUS_PROTOCOL_ID, 0x0000);
}

// ── Ports ──────────────────────────────────────────────────────────

#[test]
fn spec_tcp_guide_4_2_tcp_port() {
    assert_eq!(MODBUS_TCP_PORT, 502);
}

#[test]
fn spec_security_5_tls_port() {
    assert_eq!(MODBUS_TLS_PORT, 802);
}

// ── Unit IDs ───────────────────────────────────────────────────────

#[test]
fn spec_unit_id_broadcast() {
    assert_eq!(UNIT_ID_BROADCAST, 0x00);
}

#[test]
fn spec_tcp_guide_4_4_1_2_unit_id_tcp_device() {
    assert_eq!(UNIT_ID_TCP_DEVICE, 0xFF);
}

#[test]
fn spec_unit_id_slave_range() {
    assert_eq!(UNIT_ID_MIN_SLAVE, 1);
    assert_eq!(UNIT_ID_MAX_SLAVE, 247);
}

// ── Quantity limits per function code ──────────────────────────────

#[test]
fn spec_6_1_max_read_coils() {
    assert_eq!(MAX_READ_COILS, 2000); // 0x07D0
}

#[test]
fn spec_6_2_max_read_discrete_inputs() {
    assert_eq!(MAX_READ_DISCRETE_INPUTS, 2000); // 0x07D0
}

#[test]
fn spec_6_3_max_read_registers() {
    assert_eq!(MAX_READ_REGISTERS, 125); // 0x007D
}

#[test]
fn spec_6_11_max_write_coils() {
    assert_eq!(MAX_WRITE_COILS, 1968); // 0x07B0
}

#[test]
fn spec_6_12_max_write_registers() {
    assert_eq!(MAX_WRITE_REGISTERS, 123); // 0x007B
}

#[test]
fn spec_6_17_max_rw_read_registers() {
    assert_eq!(MAX_RW_READ_REGISTERS, 125); // 0x007D
}

#[test]
fn spec_6_17_max_rw_write_registers() {
    assert_eq!(MAX_RW_WRITE_REGISTERS, 121); // 0x0079
}

#[test]
fn spec_6_18_max_fifo_values() {
    assert_eq!(MAX_FIFO_VALUES, 31);
}

// ── Transaction limits ────────────────────────────────────────────

#[test]
fn spec_tcp_guide_4_4_1_2_max_client_transactions() {
    assert_eq!(MAX_CLIENT_TRANSACTIONS, 16);
}
