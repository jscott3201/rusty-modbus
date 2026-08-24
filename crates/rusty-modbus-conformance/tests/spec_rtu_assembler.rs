//! Public-contract tests for timestamp-driven RTU frame assembly.
//!
//! These tests supply synthetic per-byte timestamps directly. They do not use
//! `SerialTransport` and provide no evidence about serial-driver timestamp
//! quality or operating-system read boundaries.

use std::time::Duration;

use rusty_modbus_frame::{crc16, verify_crc};
use rusty_modbus_rtu::{
    AssemblerDiscardReason, AssemblerError, AssemblerOutcome, AssemblerRecovery, AssemblerState,
    RtuFrameAssembler, RtuTimestamp, RtuTiming,
};
use rusty_modbus_types::MAX_RTU_ADU_SIZE;

const T1_5: u64 = 10;
const T3_5: u64 = 20;

fn assembler() -> RtuFrameAssembler {
    RtuFrameAssembler::new(
        RtuTiming::new(Duration::from_nanos(T1_5), Duration::from_nanos(T3_5)).unwrap(),
    )
}

fn valid_adu(data: &[u8]) -> Vec<u8> {
    let mut adu = data.to_vec();
    adu.extend_from_slice(&crc16(data).to_le_bytes());
    adu
}

fn feed(assembler: &mut RtuFrameAssembler, bytes: &[u8], start: u64) -> RtuTimestamp {
    let mut timestamp = start;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if index != 0 {
            timestamp += 1;
        }
        assert_eq!(
            assembler
                .observe_byte(RtuTimestamp::from_nanos(timestamp), byte)
                .unwrap(),
            AssemblerOutcome::Progress
        );
    }
    RtuTimestamp::from_nanos(timestamp)
}

#[test]
fn t3_5_boundary_emits_complete_adu_and_retains_new_byte() {
    let adu = valid_adu(&[1, 3, 0, 0, 0, 1]);
    let mut assembler = assembler();
    let last = feed(&mut assembler, &adu, 0);
    let obsolete = assembler.next_deadline().unwrap();
    let boundary = RtuTimestamp::from_nanos(last.as_nanos() + T3_5);

    let AssemblerOutcome::FrameReady(frame) = assembler.observe_byte(boundary, 0xAA).unwrap()
    else {
        panic!("valid candidate did not complete at t3.5");
    };
    assert_eq!(frame.as_bytes(), adu);
    assert!(verify_crc(frame.as_bytes()));
    assert_eq!(assembler.state(), AssemblerState::Collecting);
    assert_eq!(assembler.candidate_len(), 1);

    assert_eq!(
        assembler
            .observe_deadline(obsolete, obsolete.due_at())
            .unwrap(),
        AssemblerOutcome::StaleDeadline
    );
    assert_eq!(assembler.candidate_len(), 1);
}

#[test]
fn t1_5_boundary_is_inclusive_and_later_byte_quarantines() {
    let mut assembler = assembler();
    assembler
        .observe_byte(RtuTimestamp::from_nanos(0), 1)
        .unwrap();
    assert_eq!(
        assembler
            .observe_byte(RtuTimestamp::from_nanos(T1_5), 3)
            .unwrap(),
        AssemblerOutcome::Progress
    );
    assert_eq!(assembler.candidate_len(), 2);

    assert_eq!(
        assembler
            .observe_byte(RtuTimestamp::from_nanos(T1_5 + T1_5 + 1), 0)
            .unwrap(),
        AssemblerOutcome::Discarded(AssemblerDiscardReason::InterCharacterGap)
    );
    assert_eq!(assembler.state(), AssemblerState::Quarantined);
    assert_eq!(assembler.candidate_len(), 0);
}

#[test]
fn quarantine_tracks_latest_noise_before_recovery() {
    let mut assembler = assembler();
    assembler
        .observe_byte(RtuTimestamp::from_nanos(0), 1)
        .unwrap();
    assembler
        .observe_byte(RtuTimestamp::from_nanos(T1_5 + 1), 2)
        .unwrap();
    let first_deadline = assembler.next_deadline().unwrap();
    let noise_at = T1_5 + 2;
    assembler
        .observe_byte(RtuTimestamp::from_nanos(noise_at), 3)
        .unwrap();

    assert_eq!(
        assembler
            .observe_deadline(first_deadline, first_deadline.due_at())
            .unwrap(),
        AssemblerOutcome::StaleDeadline
    );
    assert_eq!(assembler.state(), AssemblerState::Quarantined);
    assert_eq!(
        assembler
            .observe_byte(RtuTimestamp::from_nanos(noise_at + T3_5), 4)
            .unwrap(),
        AssemblerOutcome::Recovered(AssemblerRecovery::CandidateStarted)
    );
    assert_eq!(assembler.candidate_len(), 1);
}

#[test]
fn complete_candidate_crc_does_not_scan_valid_prefixes() {
    let mut bytes = valid_adu(&[1, 3]);
    assert!(verify_crc(&bytes));
    bytes.push(0xAA);
    assert!(!verify_crc(&bytes));
    let mut assembler = assembler();
    feed(&mut assembler, &bytes, 0);
    let deadline = assembler.next_deadline().unwrap();

    assert_eq!(
        assembler
            .observe_deadline(deadline, deadline.due_at())
            .unwrap(),
        AssemblerOutcome::Discarded(AssemblerDiscardReason::CrcMismatch)
    );
}

#[test]
fn overlength_enters_quarantine_without_retaining_violating_byte() {
    let mut assembler = assembler();
    for timestamp in 0..MAX_RTU_ADU_SIZE as u64 {
        assembler
            .observe_byte(RtuTimestamp::from_nanos(timestamp), 0x55)
            .unwrap();
    }

    assert_eq!(
        assembler
            .observe_byte(RtuTimestamp::from_nanos(MAX_RTU_ADU_SIZE as u64), 0x55)
            .unwrap(),
        AssemblerOutcome::Discarded(AssemblerDiscardReason::Overlength)
    );
    assert_eq!(assembler.state(), AssemblerState::Quarantined);
    assert_eq!(assembler.candidate_len(), 0);
    assert_eq!(assembler.diagnostics().bytes_observed, 257);
}

#[test]
fn early_and_regressing_events_preserve_active_candidate() {
    let adu = valid_adu(&[1, 0]);
    let mut assembler = assembler();
    feed(&mut assembler, &adu, 100);
    let deadline = assembler.next_deadline().unwrap();
    let len = assembler.candidate_len();

    assert!(matches!(
        assembler.observe_deadline(
            deadline,
            RtuTimestamp::from_nanos(deadline.due_at().as_nanos() - 1)
        ),
        Err(AssemblerError::DeadlineNotDue { .. })
    ));
    assert!(matches!(
        assembler.observe_byte(RtuTimestamp::from_nanos(0), 0xFF),
        Err(AssemblerError::TimestampRegression { .. })
    ));
    assert_eq!(assembler.candidate_len(), len);
    assert_eq!(assembler.next_deadline(), Some(deadline));

    let AssemblerOutcome::FrameReady(frame) = assembler
        .observe_deadline(deadline, deadline.due_at())
        .unwrap()
    else {
        panic!("transactional errors changed the valid candidate");
    };
    assert_eq!(frame.as_bytes(), adu);
}
