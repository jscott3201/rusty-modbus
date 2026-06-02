//! Wire-format headers for Modbus framing.

use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned, network_endian::U16};

use crate::constants::{MAX_PDU_SIZE, MBAP_HEADER_LEN};

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
    /// Try to construct a new MBAP header.
    ///
    /// `pdu_length` is the size of the PDU that follows (not including the unit ID byte).
    /// The `length` field is set to `pdu_length + 1` to account for the unit ID byte.
    ///
    /// Returns `None` when `pdu_length` exceeds the maximum Modbus PDU size.
    #[must_use]
    pub fn try_new(transaction_id: u16, unit_id: u8, pdu_length: u16) -> Option<Self> {
        if usize::from(pdu_length) > MAX_PDU_SIZE {
            return None;
        }

        let length = pdu_length.checked_add(1)?;
        Some(Self {
            transaction_id: U16::new(transaction_id),
            protocol_id: U16::new(crate::constants::MODBUS_PROTOCOL_ID),
            length: U16::new(length),
            unit_id,
        })
    }

    /// Construct a new MBAP header.
    ///
    /// `pdu_length` is the size of the PDU that follows (not including the unit ID byte).
    /// The `length` field is set to `pdu_length + 1` to account for the unit ID byte.
    ///
    /// # Panics
    ///
    /// Panics when `pdu_length` exceeds the maximum Modbus PDU size. Use
    /// [`Self::try_new`] when the length comes from external input.
    #[must_use]
    pub fn new(transaction_id: u16, unit_id: u8, pdu_length: u16) -> Self {
        Self::try_new(transaction_id, unit_id, pdu_length)
            .expect("pdu_length must be <= MAX_PDU_SIZE")
    }

    /// Byte count of the PDU that follows (length field minus 1 for unit ID).
    #[must_use]
    pub fn pdu_length(&self) -> u16 {
        self.length.get().saturating_sub(1)
    }

    /// Total ADU wire size in bytes.
    ///
    /// The ADU is `txn_id(2) + proto_id(2) + length_field(2) + [length bytes]`.
    /// The first 6 bytes precede the length-field payload, so total = 6 + length.
    /// We express this as `MBAP_HEADER_LEN - 1` because the header constant (7)
    /// includes the `unit_id` byte, which the length field also counts.
    #[must_use]
    pub fn adu_length(&self) -> usize {
        (MBAP_HEADER_LEN - 1) + self.length.get() as usize
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
    fn try_new_accepts_max_pdu_size() {
        let h = MbapHeader::try_new(1, 1, 253).expect("max PDU size should be accepted");
        assert_eq!(h.length.get(), 254);
        assert_eq!(h.pdu_length(), 253);
    }

    #[test]
    fn try_new_rejects_oversized_pdu() {
        assert!(MbapHeader::try_new(1, 1, 254).is_none());
        assert!(MbapHeader::try_new(1, 1, u16::MAX).is_none());
    }

    #[test]
    #[should_panic(expected = "pdu_length must be <= MAX_PDU_SIZE")]
    fn new_panics_on_oversized_pdu() {
        let _ = MbapHeader::new(1, 1, 254);
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
        // txn(2) + proto(2) + len(2) + unit(1) + pdu(5) = 12
        // Or equivalently: (MBAP_HEADER_LEN - 1) + length_field(6) = 12
        assert_eq!(h.adu_length(), 12);
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
        let h = MbapHeader::new(0x0102, 0xAB, 0x00FC);
        let bytes = h.as_bytes();

        // transaction_id: 0x0102 big-endian → [0x01, 0x02]
        assert_eq!(bytes[0], 0x01);
        assert_eq!(bytes[1], 0x02);
        // protocol_id: 0x0000 → [0x00, 0x00]
        assert_eq!(bytes[2], 0x00);
        assert_eq!(bytes[3], 0x00);
        // length: 0x00FC + 1 = 0x00FD → [0x00, 0xFD]
        assert_eq!(bytes[4], 0x00);
        assert_eq!(bytes[5], 0xFD);
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
