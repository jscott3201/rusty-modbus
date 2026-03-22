//! MBAP ↔ RTU frame translation.

use bytes::Bytes;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_types::{MbapHeader, TransactionId};

/// Convert an MBAP (TCP) frame to an RTU frame for serial forwarding.
///
/// Strips the MBAP header and creates an RTU frame with the unit ID from the header.
#[must_use]
pub fn mbap_to_rtu(frame: &Frame) -> Frame {
    let unit_id = frame.unit_id();
    Frame {
        header: FrameHeader::Rtu { unit_id },
        pdu: frame.pdu.clone(),
    }
}

/// Convert an RTU response frame back to an MBAP (TCP) frame.
///
/// Adds an MBAP header with the original transaction ID and unit ID.
#[must_use]
pub fn rtu_to_mbap(rtu_frame: &Frame, transaction_id: TransactionId, unit_id: u8) -> Frame {
    let header = MbapHeader::new(
        transaction_id.0,
        unit_id,
        u16::try_from(rtu_frame.pdu.len()).unwrap_or(u16::MAX),
    );
    Frame {
        header: FrameHeader::Mbap(header),
        pdu: rtu_frame.pdu.clone(),
    }
}

/// Build an MBAP exception response frame.
#[must_use]
pub fn make_exception_frame(
    transaction_id: TransactionId,
    unit_id: u8,
    function_code: u8,
    exception_code: u8,
) -> Frame {
    let pdu = Bytes::from(vec![function_code | 0x80, exception_code]);
    let header = MbapHeader::new(transaction_id.0, unit_id, 2);
    Frame {
        header: FrameHeader::Mbap(header),
        pdu,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mbap_to_rtu_preserves_pdu() {
        let pdu = Bytes::from_static(&[0x03, 0x00, 0x00, 0x00, 0x01]);
        let header = MbapHeader::new(42, 0x01, 5);
        let mbap = Frame {
            header: FrameHeader::Mbap(header),
            pdu: pdu.clone(),
        };

        let rtu = mbap_to_rtu(&mbap);
        assert_eq!(rtu.unit_id(), 0x01);
        assert_eq!(rtu.pdu, pdu);
    }

    #[test]
    fn rtu_to_mbap_sets_transaction_id() {
        let pdu = Bytes::from_static(&[0x03, 0x02, 0x12, 0x34]);
        let rtu = Frame {
            header: FrameHeader::Rtu { unit_id: 0x05 },
            pdu,
        };

        let mbap = rtu_to_mbap(&rtu, TransactionId(99), 0x05);
        match mbap.header {
            FrameHeader::Mbap(h) => {
                assert_eq!(h.transaction_id.get(), 99);
                assert_eq!(h.unit_id, 0x05);
            }
            FrameHeader::Rtu { .. } => panic!("expected MBAP header"),
        }
    }

    #[test]
    fn exception_frame_format() {
        let frame = make_exception_frame(TransactionId(1), 0x01, 0x03, 0x0B);
        assert_eq!(&frame.pdu[..], &[0x83, 0x0B]);
    }
}
