//! Server-path conformance for the eight function codes whose codec already
//! existed but which the server gained dispatch for: File Record (0x14/0x15),
//! FIFO Queue (0x18), and the serial-line diagnostics family (0x07, 0x08, 0x0B,
//! 0x0C, 0x11).
//!
//! These drive `handler::process_request` end-to-end against a real store, so
//! the bytes a caller actually observes are verified — request decode, store
//! access, and response encode. Worked examples come from Modbus V1.1b3 §6.14,
//! §6.15, and §6.18.

use rusty_modbus_server::DataStore;
use rusty_modbus_server::handler::process_request;
use rusty_modbus_server::{DeviceIdentification, InMemoryStore, StoreConfig};
use rusty_modbus_types::{ExceptionCode, UnitId};

const UNIT: UnitId = UnitId(1);

/// A small store keeps register-table addressing unambiguous; files and FIFO
/// queues are seeded explicitly and do not depend on the table sizes.
fn store() -> InMemoryStore {
    InMemoryStore::new(StoreConfig {
        coil_count: 100,
        discrete_input_count: 100,
        holding_register_count: 100,
        input_register_count: 100,
    })
}

/// Drive the real server path for a unit-addressed (non-broadcast) request.
async fn respond<S: DataStore>(store: &S, pdu: &[u8]) -> Vec<u8> {
    process_request(pdu, UNIT, store, &DeviceIdentification::default())
        .await
        .expect("a non-broadcast request must produce a response")
}

/// Drive the server path for an arbitrary unit (used for broadcast = unit 0).
async fn respond_opt<S: DataStore>(store: &S, pdu: &[u8], unit: UnitId) -> Option<Vec<u8>> {
    process_request(pdu, unit, store, &DeviceIdentification::default()).await
}

// ── FIFO Queue (FC 0x18) ──────────────────────────────────────────

#[tokio::test]
async fn fifo_read_matches_spec_example() {
    // Spec §6.18 p.40: FIFO at pointer 1246 (0x04DE) holds 440 (0x01B8), 4740 (0x1284).
    let s = store();
    s.set_fifo_queue(0x04DE, vec![0x01B8, 0x1284]);
    let resp = respond(&s, &[0x18, 0x04, 0xDE]).await;
    assert_eq!(
        resp,
        vec![0x18, 0x00, 0x06, 0x00, 0x02, 0x01, 0xB8, 0x12, 0x84]
    );
}

#[tokio::test]
async fn fifo_empty_queue_returns_count_zero() {
    let s = store();
    s.set_fifo_queue(0x0010, vec![]);
    // byte_count = 2 (the fifo_count field), fifo_count = 0, no data.
    assert_eq!(
        respond(&s, &[0x18, 0x00, 0x10]).await,
        vec![0x18, 0x00, 0x02, 0x00, 0x00]
    );
}

#[tokio::test]
async fn fifo_boundary_31_values_ok() {
    let s = store();
    let values: Vec<u16> = (0..31).collect();
    s.set_fifo_queue(0x0020, values);
    let resp = respond(&s, &[0x18, 0x00, 0x20]).await;
    assert_eq!(resp[0], 0x18);
    assert_eq!(&resp[1..3], &[0x00, 0x40], "byte_count = 2 + 31*2 = 0x0040");
    assert_eq!(&resp[3..5], &[0x00, 0x1F], "fifo_count = 31 = 0x001F");
    assert_eq!(resp.len(), 1 + 2 + 2 + 31 * 2);
}

#[tokio::test]
async fn fifo_over_31_values_is_illegal_data_value() {
    let s = store();
    let values: Vec<u16> = (0..32).collect();
    s.set_fifo_queue(0x0030, values);
    assert_eq!(respond(&s, &[0x18, 0x00, 0x30]).await, vec![0x98, 0x03]);
}

#[tokio::test]
async fn fifo_unknown_address_is_illegal_data_address() {
    let s = store();
    assert_eq!(respond(&s, &[0x18, 0x12, 0x34]).await, vec![0x98, 0x02]);
}

#[tokio::test]
async fn fifo_read_is_non_destructive() {
    let s = store();
    s.set_fifo_queue(0x0001, vec![0xAAAA, 0xBBBB]);
    let first = respond(&s, &[0x18, 0x00, 0x01]).await;
    let second = respond(&s, &[0x18, 0x00, 0x01]).await;
    assert_eq!(first, second, "reading a FIFO must not drain it (§6.18)");
}

#[tokio::test]
async fn fifo_broadcast_produces_no_response() {
    let s = store();
    s.set_fifo_queue(0x0001, vec![1, 2]);
    assert!(
        respond_opt(&s, &[0x18, 0x00, 0x01], UnitId(0))
            .await
            .is_none()
    );
}

// ── File Record read (FC 0x14) ────────────────────────────────────

#[tokio::test]
async fn file_read_two_groups_matches_spec_example() {
    // Spec §6.14 p.33: read file 4 record 1 (len 2) and file 3 record 9 (len 2).
    let s = store();
    s.set_file_record(4, 1, 0x0DFE);
    s.set_file_record(4, 2, 0x0020);
    s.set_file_record(3, 9, 0x33CD);
    s.set_file_record(3, 10, 0x0040);
    let req = [
        0x14, 0x0E, // FC, byte count
        0x06, 0x00, 0x04, 0x00, 0x01, 0x00, 0x02, // group 1
        0x06, 0x00, 0x03, 0x00, 0x09, 0x00, 0x02, // group 2
    ];
    // Each sub-response's File Resp Length (0x05) = 1 ref byte + 4 data bytes,
    // excluding itself; the top-level byte_count (0x0C) = (1+5)+(1+5).
    assert_eq!(
        respond(&s, &req).await,
        vec![
            0x14, 0x0C, //
            0x05, 0x06, 0x0D, 0xFE, 0x00, 0x20, //
            0x05, 0x06, 0x33, 0xCD, 0x00, 0x40,
        ]
    );
}

#[tokio::test]
async fn file_read_bad_reference_type_is_illegal_data_address() {
    let s = store();
    s.set_file_record(4, 1, 0x1111);
    // reference type 0x07 instead of the required 0x06
    let req = [0x14, 0x07, 0x07, 0x00, 0x04, 0x00, 0x01, 0x00, 0x01];
    assert_eq!(respond(&s, &req).await, vec![0x94, 0x02]);
}

#[tokio::test]
async fn file_read_bad_byte_count_is_illegal_data_value() {
    let s = store();
    // 6 sub-request bytes — not a multiple of 7
    let req = [0x14, 0x06, 0x06, 0x00, 0x04, 0x00, 0x01, 0x00];
    assert_eq!(respond(&s, &req).await, vec![0x94, 0x03]);
}

#[tokio::test]
async fn file_read_out_of_range_record_is_illegal_data_address() {
    let s = store();
    s.set_file_record(4, 1, 0x1111); // file 4 holds records 0..=1
    // start record 5, length 2 — beyond the file
    let req = [0x14, 0x07, 0x06, 0x00, 0x04, 0x00, 0x05, 0x00, 0x02];
    assert_eq!(respond(&s, &req).await, vec![0x94, 0x02]);
}

// ── File Record write (FC 0x15) ───────────────────────────────────

#[tokio::test]
async fn file_write_echoes_request_and_persists() {
    // Spec §6.15 p.35: write 3 registers to file 4, record 7.
    let s = store();
    let req = [
        0x15, 0x0D, //
        0x06, 0x00, 0x04, 0x00, 0x07, 0x00, 0x03, //
        0x06, 0xAF, 0x04, 0xBE, 0x10, 0x0D,
    ];
    assert_eq!(
        respond(&s, &req).await,
        req.to_vec(),
        "write echoes verbatim"
    );

    // Read the three registers back.
    let read = [0x14, 0x07, 0x06, 0x00, 0x04, 0x00, 0x07, 0x00, 0x03];
    assert_eq!(
        respond(&s, &read).await,
        vec![0x14, 0x08, 0x07, 0x06, 0x06, 0xAF, 0x04, 0xBE, 0x10, 0x0D]
    );
}

#[tokio::test]
async fn file_write_broadcast_produces_no_response() {
    let s = store();
    let req = [
        0x15, 0x0D, //
        0x06, 0x00, 0x04, 0x00, 0x07, 0x00, 0x03, //
        0x06, 0xAF, 0x04, 0xBE, 0x10, 0x0D,
    ];
    assert!(respond_opt(&s, &req, UnitId(0)).await.is_none());
}

// ── Diagnostics family (FC 0x07, 0x08, 0x0B, 0x0C, 0x11) ───────────

#[tokio::test]
async fn fc07_read_exception_status() {
    let s = store();
    s.set_exception_status(0x6D); // spec §6.7 example value
    assert_eq!(respond(&s, &[0x07]).await, vec![0x07, 0x6D]);
}

#[tokio::test]
async fn fc08_return_query_data_loops_back() {
    let s = store();
    assert_eq!(
        respond(&s, &[0x08, 0x00, 0x00, 0xA5, 0x37]).await,
        vec![0x08, 0x00, 0x00, 0xA5, 0x37]
    );
}

#[tokio::test]
async fn fc08_unsupported_subfunction_is_illegal_function() {
    // 0x000B (Return Bus Message Count) is a *known* sub-function, but the
    // default store does not serve it → IllegalFunction (0x01), per Figure 18 —
    // NOT IllegalDataValue.
    let s = store();
    assert_eq!(
        respond(&s, &[0x08, 0x00, 0x0B, 0x00, 0x00]).await,
        vec![0x88, 0x01]
    );
}

#[tokio::test]
async fn fc08_unknown_subcode_is_illegal_function() {
    // 0x0005 is outside the sub-function enum; decode rejects it and the handler
    // maps the resulting error to IllegalFunction (the decode-path fix).
    let s = store();
    assert_eq!(
        respond(&s, &[0x08, 0x00, 0x05, 0x00, 0x00]).await,
        vec![0x88, 0x01]
    );
}

#[tokio::test]
async fn fc0b_get_comm_event_counter_default_is_illegal_function() {
    let s = store();
    assert_eq!(respond(&s, &[0x0B]).await, vec![0x8B, 0x01]);
}

#[tokio::test]
async fn fc0c_get_comm_event_log_default_is_illegal_function() {
    let s = store();
    assert_eq!(respond(&s, &[0x0C]).await, vec![0x8C, 0x01]);
}

#[tokio::test]
async fn fc11_report_server_id_returns_blob() {
    let s = store();
    s.set_server_id(vec![0x52, 0x4D, 0xFF]); // "RM" + run-indicator byte
    assert_eq!(
        respond(&s, &[0x11]).await,
        vec![0x11, 0x03, 0x52, 0x4D, 0xFF]
    );
}

#[tokio::test]
async fn fc11_byte_count_equals_data_len() {
    // The default blob is multi-byte; the encoder panics if byte_count != len,
    // so this also guards the derive-don't-trust rule.
    let s = store();
    let resp = respond(&s, &[0x11]).await;
    assert_eq!(resp[0], 0x11);
    assert_eq!(usize::from(resp[1]), resp.len() - 2);
    assert_eq!(&resp[2..], b"rusty-modbus\xFF");
}

#[tokio::test]
async fn diagnostics_family_broadcast_produces_no_response() {
    let s = store();
    assert!(respond_opt(&s, &[0x07], UnitId(0)).await.is_none());
    assert!(respond_opt(&s, &[0x11], UnitId(0)).await.is_none());
}

// ── Default-method guard ──────────────────────────────────────────
//
// A store that implements only the four mandatory data tables and overrides
// none of the new capability methods MUST still return spec-correct codes via
// the DataStore trait defaults. This is the safety net for the public-trait
// default-method design.

struct NoCapabilityStore;

impl DataStore for NoCapabilityStore {
    async fn read_coils(&self, _: u16, _: u16, _: &mut [bool]) -> Result<usize, ExceptionCode> {
        Err(ExceptionCode::IllegalDataAddress)
    }
    async fn write_coil(&self, _: u16, _: bool) -> Result<(), ExceptionCode> {
        Err(ExceptionCode::IllegalDataAddress)
    }
    async fn write_coils(&self, _: u16, _: &[bool]) -> Result<(), ExceptionCode> {
        Err(ExceptionCode::IllegalDataAddress)
    }
    async fn read_discrete_inputs(
        &self,
        _: u16,
        _: u16,
        _: &mut [bool],
    ) -> Result<usize, ExceptionCode> {
        Err(ExceptionCode::IllegalDataAddress)
    }
    async fn read_holding_registers(
        &self,
        _: u16,
        _: u16,
        _: &mut [u16],
    ) -> Result<usize, ExceptionCode> {
        Err(ExceptionCode::IllegalDataAddress)
    }
    async fn write_register(&self, _: u16, _: u16) -> Result<(), ExceptionCode> {
        Err(ExceptionCode::IllegalDataAddress)
    }
    async fn write_registers(&self, _: u16, _: &[u16]) -> Result<(), ExceptionCode> {
        Err(ExceptionCode::IllegalDataAddress)
    }
    async fn read_input_registers(
        &self,
        _: u16,
        _: u16,
        _: &mut [u16],
    ) -> Result<usize, ExceptionCode> {
        Err(ExceptionCode::IllegalDataAddress)
    }
}

#[tokio::test]
async fn trait_defaults_report_spec_correct_codes() {
    let s = NoCapabilityStore;

    // FIFO has a meaningful address → IllegalDataAddress (0x02).
    assert_eq!(respond(&s, &[0x18, 0x00, 0x01]).await, vec![0x98, 0x02]);

    // Everything else is an unimplemented capability → IllegalFunction (0x01).
    let file_read = [0x14, 0x07, 0x06, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01];
    assert_eq!(respond(&s, &file_read).await, vec![0x94, 0x01]);
    let file_write = [
        0x15, 0x09, 0x06, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0xAA, 0xBB,
    ];
    assert_eq!(respond(&s, &file_write).await, vec![0x95, 0x01]);
    assert_eq!(respond(&s, &[0x07]).await, vec![0x87, 0x01]);
    assert_eq!(respond(&s, &[0x0B]).await, vec![0x8B, 0x01]);
    assert_eq!(respond(&s, &[0x0C]).await, vec![0x8C, 0x01]);
    assert_eq!(respond(&s, &[0x11]).await, vec![0x91, 0x01]);
    assert_eq!(
        respond(&s, &[0x08, 0x00, 0x0B, 0x00, 0x00]).await,
        vec![0x88, 0x01]
    );

    // Return Query Data still loops back even on the bare default.
    assert_eq!(
        respond(&s, &[0x08, 0x00, 0x00, 0x12, 0x34]).await,
        vec![0x08, 0x00, 0x00, 0x12, 0x34]
    );
}
