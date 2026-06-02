//! Server-path conformance for the eight function codes whose codec already
//! existed but which the server gained dispatch for: File Record (0x14/0x15),
//! FIFO Queue (0x18), and the serial-line diagnostics family (0x07, 0x08, 0x0B,
//! 0x0C, 0x11).
//!
//! These drive `handler::process_request` end-to-end against a real store, so
//! the bytes a caller actually observes are verified — request decode, store
//! access, and response encode. Worked examples come from Modbus V1.1b3 §6.14,
//! §6.15, and §6.18.

use rusty_modbus_server::handler::process_request;
use rusty_modbus_server::{
    CommEventLog, DataStore, DeviceIdentification, InMemoryStore, StoreConfig,
};
use rusty_modbus_types::{DiagnosticSubFunction, ExceptionCode, MAX_PDU_SIZE, UnitId};

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
    s.set_file_record(4, 1, 0x0DFE).unwrap();
    s.set_file_record(4, 2, 0x0020).unwrap();
    s.set_file_record(3, 9, 0x33CD).unwrap();
    s.set_file_record(3, 10, 0x0040).unwrap();
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
async fn file_read_single_register_uses_minimal_valid_response() {
    let s = store();
    s.set_file_record(4, 1, 0x1234).unwrap();
    let req = [0x14, 0x07, 0x06, 0x00, 0x04, 0x00, 0x01, 0x00, 0x01];
    assert_eq!(
        respond(&s, &req).await,
        vec![0x14, 0x04, 0x03, 0x06, 0x12, 0x34]
    );
}

#[tokio::test]
async fn file_read_bad_reference_type_is_illegal_data_address() {
    let s = store();
    s.set_file_record(4, 1, 0x1111).unwrap();
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
    s.set_file_record(4, 1, 0x1111).unwrap(); // file 4 holds records 0..=1
    // start record 5, length 2 — beyond the file
    let req = [0x14, 0x07, 0x06, 0x00, 0x04, 0x00, 0x05, 0x00, 0x02];
    assert_eq!(respond(&s, &req).await, vec![0x94, 0x02]);
}

#[tokio::test]
async fn file_read_file_zero_is_illegal_data_address() {
    let s = store();
    let req = [0x14, 0x07, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01];
    assert_eq!(respond(&s, &req).await, vec![0x94, 0x02]);
}

#[tokio::test]
async fn file_read_record_above_spec_max_is_illegal_data_address() {
    let s = store();
    let req = [0x14, 0x07, 0x06, 0x00, 0x01, 0x27, 0x10, 0x00, 0x01];
    assert_eq!(respond(&s, &req).await, vec![0x94, 0x02]);
}

#[tokio::test]
async fn file_read_record_range_crossing_spec_max_is_illegal_data_address() {
    let s = store();
    let req = [0x14, 0x07, 0x06, 0x00, 0x01, 0x27, 0x0F, 0x00, 0x02];
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

#[tokio::test]
async fn file_write_file_zero_is_illegal_data_address() {
    let s = store();
    let req = [
        0x15, 0x09, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x12, 0x34,
    ];
    assert_eq!(respond(&s, &req).await, vec![0x95, 0x02]);
}

#[tokio::test]
async fn file_write_record_above_spec_max_is_illegal_data_address() {
    let s = store();
    let req = [
        0x15, 0x09, 0x06, 0x00, 0x01, 0x27, 0x10, 0x00, 0x01, 0x12, 0x34,
    ];
    assert_eq!(respond(&s, &req).await, vec![0x95, 0x02]);
}

#[tokio::test]
async fn file_write_record_range_crossing_spec_max_is_illegal_data_address() {
    let s = store();
    let req = [
        0x15, 0x0B, 0x06, 0x00, 0x01, 0x27, 0x0F, 0x00, 0x02, 0x12, 0x34, 0x56, 0x78,
    ];
    assert_eq!(respond(&s, &req).await, vec![0x95, 0x02]);
}

#[tokio::test]
async fn file_write_zero_record_length_is_illegal_data_address() {
    let s = store();
    let req = [
        0x15, 0x09, 0x06, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x12, 0x34,
    ];
    assert_eq!(respond(&s, &req).await, vec![0x95, 0x02]);
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

/// Emit trivial implementations of the eight mandatory data-table methods so a
/// test store can focus on the optional-capability methods under test.
macro_rules! stub_core_tables {
    () => {
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
    };
}

/// Overrides nothing beyond the mandatory tables — exercises the trait defaults.
struct NoCapabilityStore;
impl DataStore for NoCapabilityStore {
    stub_core_tables!();
}

/// Serves the comm-event family and a no-reply (Force Listen Only) diagnostic.
struct DiagCapableStore;
impl DataStore for DiagCapableStore {
    stub_core_tables!();

    async fn get_comm_event_counter(&self) -> Result<(u16, u16), ExceptionCode> {
        Ok((0x0000, 0x0108))
    }
    async fn get_comm_event_log(&self) -> Result<CommEventLog, ExceptionCode> {
        // Spec §6.10 example values.
        Ok(CommEventLog {
            status: 0x0000,
            event_count: 0x0108,
            message_count: 0x0121,
            events: vec![0x20, 0x00],
        })
    }
    async fn diagnostic(
        &self,
        sub_function: DiagnosticSubFunction,
        data: &[u8],
    ) -> Result<Option<Vec<u8>>, ExceptionCode> {
        match sub_function {
            DiagnosticSubFunction::ForceListenOnlyMode => Ok(None), // no reply
            DiagnosticSubFunction::ReturnQueryData => Ok(Some(data.to_vec())),
            _ => Err(ExceptionCode::IllegalFunction),
        }
    }
}

/// Returns a configured number of communication event bytes.
struct SizedEventLogStore {
    len: usize,
}
impl DataStore for SizedEventLogStore {
    stub_core_tables!();

    async fn get_comm_event_log(&self) -> Result<CommEventLog, ExceptionCode> {
        Ok(CommEventLog {
            status: 0x0000,
            event_count: 0x0001,
            message_count: 0x0002,
            events: vec![0x5A; self.len],
        })
    }
}

/// Returns a diagnostic payload of a configured length, regardless of request data.
struct SizedDiagnosticStore {
    len: usize,
}
impl DataStore for SizedDiagnosticStore {
    stub_core_tables!();

    async fn diagnostic(
        &self,
        _: DiagnosticSubFunction,
        _: &[u8],
    ) -> Result<Option<Vec<u8>>, ExceptionCode> {
        Ok(Some(vec![0x5A; self.len]))
    }
}

/// A misbehaving store that claims to have written more registers than the
/// caller's buffer holds — the handler must not panic.
struct LyingFileStore;
impl DataStore for LyingFileStore {
    stub_core_tables!();

    async fn read_file_record(
        &self,
        _: u16,
        _: u16,
        _: u16,
        _: &mut [u16],
    ) -> Result<usize, ExceptionCode> {
        Ok(999)
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

// ── Additional coverage surfaced by adversarial review ────────────

#[tokio::test]
async fn fifo_boundary_31_values_payload_matches() {
    // The boundary test above only checks the header; assert the full payload too.
    let s = store();
    let values: Vec<u16> = (0..31).collect();
    s.set_fifo_queue(0x0040, values.clone());
    let mut expected = vec![0x18, 0x00, 0x40, 0x00, 0x1F];
    for v in &values {
        expected.extend_from_slice(&v.to_be_bytes());
    }
    assert_eq!(respond(&s, &[0x18, 0x00, 0x40]).await, expected);
}

#[tokio::test]
async fn file_read_accumulated_over_pdu_cap_is_illegal_data_value() {
    let s = store();
    for r in 0..4 {
        s.set_file_record(1, r, 0x1111).unwrap();
    }
    // 30 sub-requests × 4 registers each → ~10 response bytes apiece, past the cap.
    let mut req = vec![0x14, 30 * 7];
    for _ in 0..30 {
        req.extend_from_slice(&[0x06, 0x00, 0x01, 0x00, 0x00, 0x00, 0x04]);
    }
    assert_eq!(respond(&s, &req).await, vec![0x94, 0x03]);
}

#[tokio::test]
async fn file_read_length_over_scratch_is_illegal_data_address() {
    let s = store();
    s.set_file_record(1, 199, 0x2222).unwrap(); // file 1 spans records 0..=199
    // length 200 (0xC8) exceeds the handler's 122-register scratch buffer
    let req = [0x14, 0x07, 0x06, 0x00, 0x01, 0x00, 0x00, 0x00, 0xC8];
    assert_eq!(respond(&s, &req).await, vec![0x94, 0x02]);
}

#[tokio::test]
async fn file_read_lying_store_count_is_server_device_failure() {
    // A store reporting more written registers than the buffer holds must not
    // panic the handler — it returns ServerDeviceFailure (0x04).
    let req = [0x14, 0x07, 0x06, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02];
    assert_eq!(respond(&LyingFileStore, &req).await, vec![0x94, 0x04]);
}

#[tokio::test]
async fn file_write_grows_existing_file_preserving_records() {
    let s = store();
    s.set_file_record(1, 0, 0xAAAA).unwrap();
    s.set_file_record(1, 1, 0xBBBB).unwrap();
    // write records 5..=6, leaving a 2..4 gap
    let write = [
        0x15, 0x0B, 0x06, 0x00, 0x01, 0x00, 0x05, 0x00, 0x02, 0xCC, 0xCC, 0xDD, 0xDD,
    ];
    respond(&s, &write).await;
    // read records 0..=6 back
    let read = [0x14, 0x07, 0x06, 0x00, 0x01, 0x00, 0x00, 0x00, 0x07];
    assert_eq!(
        respond(&s, &read).await,
        vec![
            0x14, 0x10, // byte_count = 1 + 1 + 14
            0x0F, 0x06, // File Resp Length = 1 + 14
            0xAA, 0xAA, 0xBB, 0xBB, // records 0,1 preserved
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // records 2..4 zero-filled
            0xCC, 0xCC, 0xDD, 0xDD, // records 5,6 written
        ]
    );
}

#[tokio::test]
async fn fc08_return_query_data_empty_payload() {
    let s = store();
    assert_eq!(
        respond(&s, &[0x08, 0x00, 0x00]).await,
        vec![0x08, 0x00, 0x00]
    );
}

#[tokio::test]
async fn fc08_diagnostic_response_payload_at_pdu_cap_is_ok() {
    let resp = respond(&SizedDiagnosticStore { len: 250 }, &[0x08, 0x00, 0x00]).await;

    assert_eq!(resp.len(), MAX_PDU_SIZE);
    assert_eq!(&resp[..3], &[0x08, 0x00, 0x00]);
    assert!(resp[3..].iter().all(|&b| b == 0x5A));
}

#[tokio::test]
async fn fc08_diagnostic_response_payload_over_pdu_cap_is_server_device_failure() {
    assert_eq!(
        respond(&SizedDiagnosticStore { len: 251 }, &[0x08, 0x00, 0x00]).await,
        vec![0x88, 0x04]
    );
}

#[tokio::test]
async fn fc08_broadcast_produces_no_response() {
    let s = store();
    assert!(
        respond_opt(&s, &[0x08, 0x00, 0x00, 0x12, 0x34], UnitId(0))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn fc0b_get_comm_event_counter_served_by_store() {
    assert_eq!(
        respond(&DiagCapableStore, &[0x0B]).await,
        vec![0x0B, 0x00, 0x00, 0x01, 0x08]
    );
}

#[tokio::test]
async fn fc0c_get_comm_event_log_matches_spec_example() {
    // §6.10: byte_count (0x08) = 6 fixed bytes + 2 event bytes; the handler derives it.
    assert_eq!(
        respond(&DiagCapableStore, &[0x0C]).await,
        vec![0x0C, 0x08, 0x00, 0x00, 0x01, 0x08, 0x01, 0x21, 0x20, 0x00]
    );
}

#[tokio::test]
async fn fc0c_event_log_boundary_64_events_ok() {
    let resp = respond(&SizedEventLogStore { len: 64 }, &[0x0C]).await;

    assert_eq!(resp.len(), 1 + 1 + 6 + 64);
    assert_eq!(resp[0], 0x0C);
    assert_eq!(resp[1], 70); // byte_count = status/event/message fields + events
    assert!(resp[8..].iter().all(|&b| b == 0x5A));
}

#[tokio::test]
async fn fc0c_event_log_over_64_events_is_server_device_failure() {
    assert_eq!(
        respond(&SizedEventLogStore { len: 65 }, &[0x0C]).await,
        vec![0x8C, 0x04]
    );
}

#[tokio::test]
async fn fc11_report_server_id_boundary_251_bytes_ok() {
    let s = store();
    s.set_server_id(vec![0x5A; 251]);

    let resp = respond(&s, &[0x11]).await;

    assert_eq!(resp.len(), MAX_PDU_SIZE);
    assert_eq!(resp[0], 0x11);
    assert_eq!(resp[1], 251);
    assert!(resp[2..].iter().all(|&b| b == 0x5A));
}

#[tokio::test]
async fn fc11_report_server_id_over_pdu_cap_is_server_device_failure() {
    let s = store();
    s.set_server_id(vec![0x5A; 252]);

    assert_eq!(respond(&s, &[0x11]).await, vec![0x91, 0x04]);
}

#[tokio::test]
async fn fc08_force_listen_only_mode_produces_no_response() {
    // The store returns Ok(None) for sub-function 0x0004 → the server emits no reply.
    assert!(
        respond_opt(&DiagCapableStore, &[0x08, 0x00, 0x04, 0x00, 0x00], UNIT)
            .await
            .is_none()
    );
}
