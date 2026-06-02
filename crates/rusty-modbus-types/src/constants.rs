//! Protocol constants derived from the Modbus specifications.

// --- PDU / ADU sizes ---

/// Maximum PDU size in bytes.
/// Spec V1.1b3 §4.1 — serial ADU 256 minus address(1) minus CRC(2).
pub const MAX_PDU_SIZE: usize = 253;

/// MBAP header length in bytes.
/// TCP Guide §3.1.3 — TxnId(2) + ProtoId(2) + Len(2) + UnitId(1).
pub const MBAP_HEADER_LEN: usize = 7;

/// Maximum TCP ADU size in bytes.
/// MBAP header + maximum PDU.
pub const MAX_TCP_ADU_SIZE: usize = MBAP_HEADER_LEN + MAX_PDU_SIZE;

/// Maximum RTU ADU size in bytes.
/// Address(1) + PDU(253) + CRC(2).
pub const MAX_RTU_ADU_SIZE: usize = 256;

/// CRC-16 length in bytes for RTU framing.
pub const RTU_CRC_LEN: usize = 2;

// --- Protocol identifiers ---

/// MBAP protocol identifier. TCP Guide §3.1.3.
pub const MODBUS_PROTOCOL_ID: u16 = 0x0000;

// --- Ports ---

/// Standard Modbus/TCP port. IANA registration, TCP Guide §4.2.
pub const MODBUS_TCP_PORT: u16 = 502;

/// Modbus/TCP Security port. Security Spec §5, IANA mbaps.
pub const MODBUS_TLS_PORT: u16 = 802;

/// Required TLS maximum fragment length for Modbus/TCP Security.
///
/// Security Spec §6.2 requires support for a 512-byte maximum fragment length.
pub const MODBUS_TLS_MAX_FRAGMENT_SIZE: usize = 512;

// --- Unit IDs ---

/// Unit ID for direct TCP device. TCP Guide §4.4.1.2.
pub const UNIT_ID_TCP_DEVICE: u8 = 0xFF;

/// Broadcast unit ID. Spec V1.1b3 §4.
pub const UNIT_ID_BROADCAST: u8 = 0x00;

/// Minimum valid slave/unit ID. Spec V1.1b3 §4.
pub const UNIT_ID_MIN_SLAVE: u8 = 1;

/// Maximum valid slave/unit ID. Spec V1.1b3 §4.
pub const UNIT_ID_MAX_SLAVE: u8 = 247;

// --- Quantity limits per function code ---

/// Maximum coils in a single read request. Spec §6.1.
pub const MAX_READ_COILS: u16 = 2000;

/// Maximum discrete inputs in a single read request. Spec §6.2.
pub const MAX_READ_DISCRETE_INPUTS: u16 = 2000;

/// Maximum registers in a single read request. Spec §6.3/6.4.
pub const MAX_READ_REGISTERS: u16 = 125;

/// Maximum coils in a single write request. Spec §6.11 (0x07B0).
pub const MAX_WRITE_COILS: u16 = 1968;

/// Maximum registers in a single write request. Spec §6.12 (0x007B).
pub const MAX_WRITE_REGISTERS: u16 = 123;

/// Maximum registers to read in a read/write multiple request. Spec §6.17.
pub const MAX_RW_READ_REGISTERS: u16 = 125;

/// Maximum registers to write in a read/write multiple request. Spec §6.17 (0x0079).
pub const MAX_RW_WRITE_REGISTERS: u16 = 121;

/// Maximum FIFO values returned. Spec §6.18.
pub const MAX_FIFO_VALUES: u16 = 31;

// --- Transaction limits ---

/// Maximum concurrent client transactions. TCP Guide §4.4.1.2.
pub const MAX_CLIENT_TRANSACTIONS: u16 = 16;

// --- Security ---

/// OID for Modbus role in x.509 certificates. Security Spec §8.4, R-21.
pub const MODBUS_ROLE_OID: &str = "1.3.6.1.4.1.50316.802.1";
