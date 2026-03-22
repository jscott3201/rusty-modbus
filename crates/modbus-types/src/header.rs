//! Wire-format headers for Modbus framing.

use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned, network_endian::U16};

use crate::constants::MBAP_HEADER_LEN;

/// The 7-byte MBAP header for Modbus/TCP.
///
/// Overlaid directly on the network buffer using `zerocopy` for zero-copy
/// decode. Fields use `network_endian::U16` so they are stored big-endian
/// on the wire. Access values via `.get()` for native-endian `u16`.
///
/// The `Unaligned` derive is required because `#[repr(packed)]` has alignment 1,
/// and zerocopy 0.8's `ref_from_prefix` requires `Unaligned` for packed structs.
// Note: no serde derive — zerocopy::network_endian::U16 does not implement
// Serialize/Deserialize. MbapHeader is a wire-format type; serialize the
// individual fields via .get() accessors when needed.
#[derive(FromBytes, IntoBytes, Immutable, KnownLayout, Unaligned, Clone, Copy, Debug)]
#[repr(C, packed)]
pub struct MbapHeader {
    /// Transaction identifier for request/response matching.
    pub transaction_id: U16,
    /// Protocol identifier — must be 0x0000 for Modbus.
    pub protocol_id: U16,
    /// Number of following bytes (unit ID + PDU).
    pub length: U16,
    /// Unit identifier (slave address).
    pub unit_id: u8,
}

impl MbapHeader {
    /// Construct a new MBAP header.
    ///
    /// `pdu_length` is the size of the PDU that follows (not including the unit ID byte).
    /// The `length` field is set to `pdu_length + 1` to account for the unit ID byte.
    #[must_use]
    pub fn new(transaction_id: u16, unit_id: u8, pdu_length: u16) -> Self {
        Self {
            transaction_id: U16::new(transaction_id),
            protocol_id: U16::new(crate::constants::MODBUS_PROTOCOL_ID),
            length: U16::new(pdu_length + 1), // +1 for unit_id byte
            unit_id,
        }
    }

    /// Byte count of the PDU that follows (length field minus 1 for unit ID).
    #[must_use]
    pub fn pdu_length(&self) -> u16 {
        self.length.get().saturating_sub(1)
    }

    /// Total ADU length including this header.
    #[must_use]
    pub fn adu_length(&self) -> usize {
        MBAP_HEADER_LEN + self.length.get() as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zerocopy::IntoBytes;

    #[test]
    fn new_sets_fields_correctly() {
        let h = MbapHeader::new(42, 1, 5);
        assert_eq!(h.transaction_id.get(), 42);
        assert_eq!(h.protocol_id.get(), 0x0000);
        assert_eq!(h.length.get(), 6); // pdu_length(5) + 1
        assert_eq!(h.unit_id, 1);
    }

    #[test]
    fn pdu_length_subtracts_unit_id() {
        let h = MbapHeader::new(0, 0, 10);
        assert_eq!(h.pdu_length(), 10);
    }

    #[test]
    fn pdu_length_saturates_at_zero() {
        // Construct a header with length=0 (degenerate case).
        let h = MbapHeader {
            transaction_id: U16::new(0),
            protocol_id: U16::new(0),
            length: U16::new(0),
            unit_id: 0,
        };
        assert_eq!(h.pdu_length(), 0);
    }

    #[test]
    fn adu_length_includes_header() {
        let h = MbapHeader::new(0, 0, 5);
        // MBAP_HEADER_LEN(7) + length field(6) = 13
        assert_eq!(h.adu_length(), 13);
    }

    #[test]
    fn round_trip_through_bytes() {
        let original = MbapHeader::new(0x1234, 0xFF, 100);

        // Write to bytes.
        let bytes = original.as_bytes();
        assert_eq!(bytes.len(), 7);

        // Read back via zerocopy.
        let restored = MbapHeader::ref_from_bytes(bytes).expect("ref_from_bytes failed");

        assert_eq!(restored.transaction_id.get(), 0x1234);
        assert_eq!(restored.protocol_id.get(), 0x0000);
        assert_eq!(restored.length.get(), 101); // 100 + 1
        assert_eq!(restored.unit_id, 0xFF);
    }

    #[test]
    fn wire_format_is_big_endian() {
        let h = MbapHeader::new(0x0102, 0xAB, 0x0304);
        let bytes = h.as_bytes();

        // transaction_id: 0x0102 big-endian → [0x01, 0x02]
        assert_eq!(bytes[0], 0x01);
        assert_eq!(bytes[1], 0x02);
        // protocol_id: 0x0000 → [0x00, 0x00]
        assert_eq!(bytes[2], 0x00);
        assert_eq!(bytes[3], 0x00);
        // length: 0x0304 + 1 = 0x0305 → [0x03, 0x05]
        assert_eq!(bytes[4], 0x03);
        assert_eq!(bytes[5], 0x05);
        // unit_id: 0xAB
        assert_eq!(bytes[6], 0xAB);
    }

    #[test]
    fn ref_from_prefix_with_trailing_data() {
        let mut buf = [0u8; 10];
        let h = MbapHeader::new(1, 2, 3);
        buf[..7].copy_from_slice(h.as_bytes());
        buf[7..].copy_from_slice(&[0xAA, 0xBB, 0xCC]);

        let (parsed, suffix) = MbapHeader::ref_from_prefix(&buf).expect("ref_from_prefix failed");
        assert_eq!(parsed.transaction_id.get(), 1);
        assert_eq!(suffix, &[0xAA, 0xBB, 0xCC]);
    }
}
