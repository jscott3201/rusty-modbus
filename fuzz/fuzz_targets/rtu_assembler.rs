#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;
use rusty_modbus_frame::verify_crc;
use rusty_modbus_rtu::{
    AssemblerDeadline, AssemblerError, AssemblerOutcome, AssemblerState, RtuFrameAssembler,
    RtuTimestamp, RtuTiming,
};
use rusty_modbus_types::MAX_RTU_ADU_SIZE;

const HEADER_SIZE: usize = 10;
const EVENT_SIZE: usize = 4;
const MAX_EVENTS: usize = 512;

fn add(base: u64, delta: u64) -> u64 {
    base.saturating_add(delta)
}

fn byte_timestamp(mode: u8, auxiliary: u8, latest: u64, t1_5: u64, t3_5: u64) -> u64 {
    match mode % 10 {
        0 => latest,
        1 => add(latest, 1),
        2 => add(latest, t1_5),
        3 => add(latest, t1_5 + 1),
        4 => add(latest, t3_5 - 1),
        5 => add(latest, t3_5),
        6 => add(latest, t3_5 + 1),
        7 => latest.saturating_sub(u64::from(auxiliary) + 1),
        8 => u64::MAX - u64::from(auxiliary),
        9 => u64::from(auxiliary),
        _ => unreachable!(),
    }
}

fn deadline_timestamp(mode: u8, auxiliary: u8, deadline: AssemblerDeadline, latest: u64) -> u64 {
    match mode % 7 {
        0 => deadline.due_at().as_nanos().saturating_sub(1),
        1 => deadline.due_at().as_nanos(),
        2 => deadline.due_at().as_nanos().saturating_add(1),
        3 => latest.saturating_sub(u64::from(auxiliary) + 1),
        4 => latest,
        5 => u64::MAX - u64::from(auxiliary),
        6 => u64::from(auxiliary),
        _ => unreachable!(),
    }
}

fn inspect_outcome(outcome: &AssemblerOutcome, valid_frames: &mut u64) {
    if let AssemblerOutcome::FrameReady(frame) = outcome {
        assert!((4..=MAX_RTU_ADU_SIZE).contains(&frame.len()));
        assert_eq!(frame.len(), frame.as_bytes().len());
        assert!(verify_crc(frame.as_bytes()));
        *valid_frames += 1;
    }
}

fn assert_state_bounds(assembler: &RtuFrameAssembler) {
    match assembler.state() {
        AssemblerState::Idle => {
            assert_eq!(assembler.candidate_len(), 0);
            assert_eq!(assembler.next_deadline(), None);
        }
        AssemblerState::Collecting => {
            assert!((1..=MAX_RTU_ADU_SIZE).contains(&assembler.candidate_len()));
            assert!(assembler.next_deadline().is_some());
        }
        AssemblerState::Quarantined => {
            assert_eq!(assembler.candidate_len(), 0);
            assert!(assembler.next_deadline().is_some());
        }
    }
}

fn assert_transactional_error(
    result: &Result<AssemblerOutcome, AssemblerError>,
    before: (AssemblerState, usize, Option<AssemblerDeadline>),
    assembler: &RtuFrameAssembler,
) {
    if result.is_err() || matches!(result, Ok(AssemblerOutcome::StaleDeadline)) {
        assert_eq!(assembler.state(), before.0);
        assert_eq!(assembler.candidate_len(), before.1);
        assert_eq!(assembler.next_deadline(), before.2);
    }
}

fuzz_target!(|input: &[u8]| {
    if input.len() < HEADER_SIZE {
        return;
    }

    let t1_5 = u64::from(input[0]) + 1;
    let t3_5 = t1_5 + u64::from(input[1]) + 1;
    let initial = u64::from_le_bytes(input[2..HEADER_SIZE].try_into().unwrap());
    let timing = RtuTiming::new(
        std::time::Duration::from_nanos(t1_5),
        std::time::Duration::from_nanos(t3_5),
    )
    .unwrap();
    let mut assembler = RtuFrameAssembler::new(timing);
    let mut latest = initial;
    let mut stale_deadline = None;
    let mut valid_frames = 0;
    let mut bytes_observed = 0;

    for event in input[HEADER_SIZE..]
        .chunks_exact(EVENT_SIZE)
        .take(MAX_EVENTS)
    {
        let before = (
            assembler.state(),
            assembler.candidate_len(),
            assembler.next_deadline(),
        );
        let operation = event[0] % 3;
        if operation == 0 {
            let timestamp = byte_timestamp(event[1], event[3], latest, t1_5, t3_5);
            bytes_observed += 1;
            let result = assembler.observe_byte(RtuTimestamp::from_nanos(timestamp), event[2]);
            assert_transactional_error(&result, before, &assembler);
            if let Ok(outcome) = &result {
                inspect_outcome(outcome, &mut valid_frames);
                latest = timestamp;
            }
        } else {
            let selected = if operation == 1 {
                assembler.next_deadline()
            } else {
                stale_deadline.or_else(|| assembler.next_deadline())
            };
            if let Some(deadline) = selected {
                let timestamp = deadline_timestamp(event[1], event[3], deadline, latest);
                let result =
                    assembler.observe_deadline(deadline, RtuTimestamp::from_nanos(timestamp));
                assert_transactional_error(&result, before, &assembler);
                if let Ok(outcome) = &result {
                    inspect_outcome(outcome, &mut valid_frames);
                    if !matches!(outcome, AssemblerOutcome::StaleDeadline) {
                        latest = timestamp;
                    }
                }
            }
        }

        let after_deadline = assembler.next_deadline();
        if before.2.is_some() && before.2 != after_deadline {
            stale_deadline = before.2;
        }
        assert_state_bounds(&assembler);
        assert_eq!(assembler.diagnostics().valid_frames, valid_frames);
        assert_eq!(assembler.diagnostics().bytes_observed, bytes_observed);
    }
});
