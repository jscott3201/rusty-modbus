//! Gateway conformance tests.
//!
//! Verifies MBAP ↔ RTU translation per TCP Guide §3.1.2 and
//! gateway exception codes per V1.1b3 §7.

use bytes::Bytes;
use modbus_frame::frame::{Frame, FrameHeader};
use modbus_gateway::routing::{RouteTable};
use modbus_gateway::translator::{make_exception_frame, mbap_to_rtu, rtu_to_mbap};
use modbus_types::{ExceptionCode, MbapHeader, TransactionId};

// ── §3.1.2 Frame Translation ──────────────────────────────────────

#[test]
fn spec_3_1_2_mbap_to_rtu_preserves_pdu() {
    // Gateway strips MBAP header, creates RTU frame with unit ID
    let pdu = Bytes::from_static(&[0x03, 0x00, 0x6B, 0x00, 0x03]);
    let mbap_frame = Frame {
        header: FrameHeader::Mbap(MbapHeader::new(42, 0x01, pdu.len() as u16)),
        pdu: pdu.clone(),
    };

    let rtu = mbap_to_rtu(&mbap_frame);
    assert_eq!(rtu.unit_id(), 0x01);
    assert_eq!(&rtu.pdu[..], &pdu[..], "PDU must be preserved during translation");
}

#[test]
fn spec_3_1_2_rtu_to_mbap_restores_transaction_id() {
    // Gateway must use the ORIGINAL transaction ID from the TCP request
    let rtu_pdu = Bytes::from_static(&[0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64]);
    let rtu_frame = Frame {
        header: FrameHeader::Rtu { unit_id: 0x01 },
        pdu: rtu_pdu.clone(),
    };

    let mbap = rtu_to_mbap(&rtu_frame, TransactionId(0x1234), 0x01);
    match mbap.header {
        FrameHeader::Mbap(h) => {
            assert_eq!(h.transaction_id.get(), 0x1234, "must use original transaction ID");
            assert_eq!(h.unit_id, 0x01);
            assert_eq!(h.protocol_id.get(), 0x0000);
        }
        _ => panic!("expected MBAP header"),
    }
    assert_eq!(&mbap.pdu[..], &rtu_pdu[..]);
}

#[test]
fn spec_3_1_2_round_trip_preserves_data() {
    // MBAP → RTU → MBAP should preserve all PDU data
    let pdu = Bytes::from_static(&[0x06, 0x00, 0x01, 0x00, 0x03]);
    let original = Frame {
        header: FrameHeader::Mbap(MbapHeader::new(99, 0x05, pdu.len() as u16)),
        pdu: pdu.clone(),
    };

    let rtu = mbap_to_rtu(&original);
    let restored = rtu_to_mbap(&rtu, TransactionId(99), 0x05);

    assert_eq!(&restored.pdu[..], &pdu[..]);
    match restored.header {
        FrameHeader::Mbap(h) => {
            assert_eq!(h.transaction_id.get(), 99);
            assert_eq!(h.unit_id, 0x05);
        }
        _ => panic!("expected MBAP"),
    }
}

// ── §7 Gateway Exception Codes ────────────────────────────────────

#[test]
fn spec_7_gateway_path_unavailable() {
    // Exception 0x0A: "indicates that the gateway was unable to allocate
    //                   an internal communication path"
    let frame = make_exception_frame(
        TransactionId(1),
        0x01,
        0x03,
        ExceptionCode::GatewayPathUnavailable.code(),
    );
    assert_eq!(&frame.pdu[..], &[0x83, 0x0A]);
}

#[test]
fn spec_7_gateway_target_failed() {
    // Exception 0x0B: "indicates that no response was obtained from the target device"
    let frame = make_exception_frame(
        TransactionId(1),
        0x01,
        0x03,
        ExceptionCode::GatewayTargetDeviceFailedToRespond.code(),
    );
    assert_eq!(&frame.pdu[..], &[0x83, 0x0B]);
}

// ── Routing Table ─────────────────────────────────────────────────

#[test]
fn routing_resolve_within_range() {
    use modbus_gateway::config::RouteEntry;
    let table = RouteTable::new(vec![
        RouteEntry {
            unit_id_range: 1..=10,
            backend_addr: "127.0.0.1:5001".parse().unwrap(),
        },
        RouteEntry {
            unit_id_range: 11..=20,
            backend_addr: "127.0.0.1:5002".parse().unwrap(),
        },
    ]);

    assert_eq!(table.resolve(1), Some("127.0.0.1:5001".parse().unwrap()));
    assert_eq!(table.resolve(10), Some("127.0.0.1:5001".parse().unwrap()));
    assert_eq!(table.resolve(11), Some("127.0.0.1:5002".parse().unwrap()));
    assert_eq!(table.resolve(20), Some("127.0.0.1:5002".parse().unwrap()));
    assert!(table.resolve(0).is_none());
    assert!(table.resolve(21).is_none());
}

#[test]
fn routing_all_backends_deduplicates() {
    use modbus_gateway::config::RouteEntry;
    let addr = "127.0.0.1:5001".parse().unwrap();
    let table = RouteTable::new(vec![
        RouteEntry { unit_id_range: 1..=10, backend_addr: addr },
        RouteEntry { unit_id_range: 11..=20, backend_addr: addr },
    ]);
    let backends: Vec<_> = table.all_backends().collect();
    assert_eq!(backends.len(), 1, "duplicate backends should be deduplicated");
}

#[test]
#[should_panic(expected = "overlapping")]
fn routing_overlapping_ranges_rejected() {
    use modbus_gateway::config::RouteEntry;
    let _ = RouteTable::new(vec![
        RouteEntry {
            unit_id_range: 1..=10,
            backend_addr: "127.0.0.1:5001".parse().unwrap(),
        },
        RouteEntry {
            unit_id_range: 5..=15,
            backend_addr: "127.0.0.1:5002".parse().unwrap(),
        },
    ]);
}
