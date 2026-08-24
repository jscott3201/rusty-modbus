//! Timestamp-driven Modbus RTU frame assembly.
//!
//! This module has no clock or I/O dependency. Callers must provide monotonic,
//! per-byte timestamps from a source that preserves wire timing. Read-completion
//! times and timestamps reconstructed from an operating-system buffer do not
//! satisfy that contract.

use std::fmt;
use std::time::Duration;

use rusty_modbus_frame::verify_crc;
use rusty_modbus_types::MAX_RTU_ADU_SIZE;
use thiserror::Error;

use crate::ResolvedRtuConfig;

const MIN_RTU_ADU_SIZE: usize = 4;

/// A monotonic timestamp expressed as nanoseconds from a caller-chosen epoch.
///
/// Zero is a valid epoch. The assembler compares timestamps only within one
/// instance and never interprets them as wall-clock time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RtuTimestamp(u64);

impl RtuTimestamp {
    /// Construct a timestamp from nanoseconds since the caller's epoch.
    #[must_use]
    pub const fn from_nanos(nanoseconds: u64) -> Self {
        Self(nanoseconds)
    }

    /// Return the timestamp as nanoseconds since the caller's epoch.
    #[must_use]
    pub const fn as_nanos(self) -> u64 {
        self.0
    }

    /// Add a duration without wrapping the nanosecond representation.
    ///
    /// Returns `None` if the duration cannot be represented as `u64`
    /// nanoseconds or the sum exceeds [`u64::MAX`].
    #[must_use]
    pub fn checked_add(self, duration: Duration) -> Option<Self> {
        let nanoseconds = u64::try_from(duration.as_nanos()).ok()?;
        self.0.checked_add(nanoseconds).map(Self)
    }

    /// Measure a nonnegative duration from `earlier` to this timestamp.
    ///
    /// Returns `None` when `earlier` is later than this timestamp.
    #[must_use]
    pub fn checked_duration_since(self, earlier: Self) -> Option<Duration> {
        self.0.checked_sub(earlier.0).map(Duration::from_nanos)
    }
}

/// Errors returned while validating assembler timing intervals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RtuTimingError {
    /// t1.5 does not fit in the assembler's nanosecond representation.
    #[error("t1.5 exceeds the u64 nanosecond range")]
    T1_5OutOfRange,
    /// t3.5 does not fit in the assembler's nanosecond representation.
    #[error("t3.5 exceeds the u64 nanosecond range")]
    T3_5OutOfRange,
    /// The intervals do not satisfy `0 < t1.5 < t3.5`.
    #[error("RTU timing must satisfy 0 < t1.5 < t3.5")]
    InvalidTiming,
}

/// Validated RTU receive timing used by [`RtuFrameAssembler`].
///
/// Both values are stored as whole nanoseconds and satisfy
/// `0 < t1.5 < t3.5`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtuTiming {
    t1_5_nanos: u64,
    t3_5_nanos: u64,
}

impl RtuTiming {
    /// Validate RTU t1.5 and t3.5 intervals.
    ///
    /// # Errors
    ///
    /// Returns [`RtuTimingError`] when either duration exceeds `u64`
    /// nanoseconds or the pair does not satisfy `0 < t1.5 < t3.5`.
    pub fn new(t1_5: Duration, t3_5: Duration) -> Result<Self, RtuTimingError> {
        let t1_5_nanos =
            u64::try_from(t1_5.as_nanos()).map_err(|_| RtuTimingError::T1_5OutOfRange)?;
        let t3_5_nanos =
            u64::try_from(t3_5.as_nanos()).map_err(|_| RtuTimingError::T3_5OutOfRange)?;
        if t1_5_nanos == 0 || t1_5_nanos >= t3_5_nanos {
            return Err(RtuTimingError::InvalidTiming);
        }
        Ok(Self {
            t1_5_nanos,
            t3_5_nanos,
        })
    }

    /// Return the validated t1.5 interval.
    #[must_use]
    pub const fn t1_5(self) -> Duration {
        Duration::from_nanos(self.t1_5_nanos)
    }

    /// Return the validated t3.5 interval.
    #[must_use]
    pub const fn t3_5(self) -> Duration {
        Duration::from_nanos(self.t3_5_nanos)
    }
}

impl TryFrom<&ResolvedRtuConfig> for RtuTiming {
    type Error = RtuTimingError;

    fn try_from(config: &ResolvedRtuConfig) -> Result<Self, Self::Error> {
        Self::new(config.t1_5(), config.t3_5())
    }
}

/// A tokenized t3.5 deadline produced by an assembler.
///
/// Pass the complete value back to [`RtuFrameAssembler::observe_deadline`]. A
/// copied older value remains safe to submit after a newer deadline replaces
/// it; the assembler reports it as stale without applying timestamp checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AssemblerDeadline {
    due_at: RtuTimestamp,
    sequence: u64,
}

impl AssemblerDeadline {
    /// Return the earliest timestamp at which this deadline is due.
    #[must_use]
    pub const fn due_at(self) -> RtuTimestamp {
        self.due_at
    }

    /// Return the assembler-local token sequence used to identify stale work.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }
}

/// Stable reasons why a complete or partial RTU candidate was discarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssemblerDiscardReason {
    /// A new byte arrived after t1.5 but before t3.5.
    InterCharacterGap,
    /// A 257th byte arrived without a t3.5 frame boundary.
    Overlength,
    /// A t3.5 boundary closed a candidate shorter than four bytes.
    TooShort,
    /// The complete candidate failed CRC-16/Modbus validation.
    CrcMismatch,
}

/// The public assembly state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssemblerState {
    /// No candidate or quarantine interval is active.
    Idle,
    /// Bytes are being retained as one candidate ADU.
    Collecting,
    /// Input is ignored until t3.5 has elapsed since the latest noise byte.
    Quarantined,
}

/// The destination reached when quarantine ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssemblerRecovery {
    /// A recovery deadline elapsed and left the assembler idle.
    Idle,
    /// A byte at or after the recovery boundary started a new candidate.
    CandidateStarted,
}

/// A fixed-capacity owned RTU ADU emitted after boundary and CRC validation.
///
/// The populated slice is between 4 and [`MAX_RTU_ADU_SIZE`] bytes. The type
/// owns inline storage and does not allocate when it is produced.
#[derive(Clone, PartialEq, Eq)]
pub struct OwnedRtuAdu {
    bytes: [u8; MAX_RTU_ADU_SIZE],
    len: usize,
}

impl OwnedRtuAdu {
    /// Return the populated ADU bytes, including the unit identifier and CRC.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    /// Return the populated ADU length.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Return whether the populated ADU is empty.
    ///
    /// Assembler-produced values are never empty; this method accompanies
    /// [`Self::len`] for slice-like use.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl AsRef<[u8]> for OwnedRtuAdu {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl fmt::Debug for OwnedRtuAdu {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("OwnedRtuAdu")
            .field(&self.as_bytes())
            .finish()
    }
}

/// Result of one accepted byte or deadline event.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    clippy::large_enum_variant,
    reason = "boxing the completed ADU would violate allocation-free event processing"
)]
pub enum AssemblerOutcome {
    /// State advanced without completing or discarding a candidate.
    Progress,
    /// A complete candidate passed the ADU length and CRC checks.
    FrameReady(OwnedRtuAdu),
    /// A candidate was rejected for the enclosed stable reason.
    Discarded(AssemblerDiscardReason),
    /// Quarantine ended at a clean t3.5 boundary.
    Recovered(AssemblerRecovery),
    /// The submitted deadline was no longer active.
    StaleDeadline,
}

/// Event errors that do not mutate assembly state or retained candidate bytes.
///
/// Diagnostic counters still record observed bytes and the applicable error
/// class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AssemblerError {
    /// An event timestamp preceded the latest accepted event timestamp.
    #[error("RTU timestamp regressed from {previous:?} to {observed:?}")]
    TimestampRegression {
        /// Latest timestamp accepted by the assembler.
        previous: RtuTimestamp,
        /// Regressing timestamp supplied by the caller.
        observed: RtuTimestamp,
    },
    /// The active deadline callback was invoked before its due time.
    #[error("RTU deadline {deadline:?} was observed early at {observed:?}")]
    DeadlineNotDue {
        /// Active deadline that has not become due.
        deadline: AssemblerDeadline,
        /// Callback timestamp supplied by the caller.
        observed: RtuTimestamp,
    },
    /// Adding t3.5 to a byte timestamp exceeded [`u64::MAX`].
    #[error("t3.5 deadline overflows after timestamp {timestamp:?}")]
    TimestampOverflow {
        /// Byte timestamp from which the deadline was being generated.
        timestamp: RtuTimestamp,
    },
    /// The deadline token sequence was exhausted.
    #[error("RTU deadline token sequence is exhausted")]
    DeadlineGenerationOverflow,
}

/// Saturating counters for assembler activity and discard classes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AssemblerDiagnostics {
    /// Byte events submitted, including byte events that return an error.
    pub bytes_observed: u64,
    /// Candidates emitted after complete-ADU CRC validation.
    pub valid_frames: u64,
    /// Candidates discarded because of a gap greater than t1.5 and below t3.5.
    pub inter_character_discards: u64,
    /// Candidates discarded when a 257th byte arrived before t3.5.
    pub overlength_discards: u64,
    /// Candidates discarded at t3.5 with fewer than four bytes.
    pub too_short_discards: u64,
    /// Complete candidates discarded after CRC mismatch.
    pub crc_mismatch_discards: u64,
    /// Deadline callbacks ignored because their token was obsolete.
    pub stale_deadlines: u64,
    /// Active deadline callbacks rejected before their due timestamp.
    pub early_deadlines: u64,
    /// Byte or active-deadline events rejected for timestamp regression.
    pub timestamp_regressions: u64,
    /// Clean t3.5 boundaries that ended quarantine.
    pub quarantine_recoveries: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    Collecting {
        len: usize,
        last_byte: RtuTimestamp,
        deadline: AssemblerDeadline,
    },
    Quarantined {
        last_noise: RtuTimestamp,
        deadline: AssemblerDeadline,
    },
}

impl State {
    const fn public(self) -> AssemblerState {
        match self {
            Self::Idle => AssemblerState::Idle,
            Self::Collecting { .. } => AssemblerState::Collecting,
            Self::Quarantined { .. } => AssemblerState::Quarantined,
        }
    }

    const fn deadline(self) -> Option<AssemblerDeadline> {
        match self {
            Self::Idle => None,
            Self::Collecting { deadline, .. } | Self::Quarantined { deadline, .. } => {
                Some(deadline)
            }
        }
    }
}

/// Fixed-buffer, event-driven Modbus RTU frame assembler.
///
/// [`Self::observe_byte`] enforces t1.5 invalidation and t3.5 boundaries from
/// caller-supplied per-byte timestamps. [`Self::observe_deadline`] handles only
/// active t3.5 deadlines; there is no t1.5 silence timer because silence alone
/// does not invalidate a candidate. Candidate validation checks the RTU ADU
/// envelope and whole-candidate CRC, not PDU function semantics.
///
/// The assembler is not connected to [`SerialTransport`](crate::SerialTransport)
/// or any async runtime. Its fixed candidate and output storage avoid heap
/// allocation during event processing.
pub struct RtuFrameAssembler {
    timing: RtuTiming,
    candidate: [u8; MAX_RTU_ADU_SIZE],
    state: State,
    last_observed: Option<RtuTimestamp>,
    deadline_sequence: u64,
    diagnostics: AssemblerDiagnostics,
}

impl fmt::Debug for RtuFrameAssembler {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RtuFrameAssembler")
            .field("timing", &self.timing)
            .field("state", &self.state)
            .field("last_observed", &self.last_observed)
            .field("deadline_sequence", &self.deadline_sequence)
            .field("diagnostics", &self.diagnostics)
            .finish_non_exhaustive()
    }
}

impl RtuFrameAssembler {
    /// Construct an idle assembler from validated timing.
    #[must_use]
    pub const fn new(timing: RtuTiming) -> Self {
        Self {
            timing,
            candidate: [0; MAX_RTU_ADU_SIZE],
            state: State::Idle,
            last_observed: None,
            deadline_sequence: 0,
            diagnostics: AssemblerDiagnostics {
                bytes_observed: 0,
                valid_frames: 0,
                inter_character_discards: 0,
                overlength_discards: 0,
                too_short_discards: 0,
                crc_mismatch_discards: 0,
                stale_deadlines: 0,
                early_deadlines: 0,
                timestamp_regressions: 0,
                quarantine_recoveries: 0,
            },
        }
    }

    /// Construct an assembler from a resolved strict RTU configuration.
    ///
    /// # Errors
    ///
    /// Returns [`RtuTimingError`] if the resolved intervals cannot be represented
    /// by the assembler. Values produced by the current strict configuration
    /// resolver satisfy these bounds.
    pub fn from_resolved(config: &ResolvedRtuConfig) -> Result<Self, RtuTimingError> {
        RtuTiming::try_from(config).map(Self::new)
    }

    /// Return the validated timing used by this assembler.
    #[must_use]
    pub const fn timing(&self) -> RtuTiming {
        self.timing
    }

    /// Return the current public assembly state.
    #[must_use]
    pub const fn state(&self) -> AssemblerState {
        self.state.public()
    }

    /// Return the number of bytes retained in the active candidate.
    ///
    /// Idle and quarantined states return zero.
    #[must_use]
    pub const fn candidate_len(&self) -> usize {
        match self.state {
            State::Collecting { len, .. } => len,
            State::Idle | State::Quarantined { .. } => 0,
        }
    }

    /// Return the active t3.5 deadline, including its stale-work token.
    #[must_use]
    pub const fn next_deadline(&self) -> Option<AssemblerDeadline> {
        self.state.deadline()
    }

    /// Return a snapshot of the saturating diagnostic counters.
    #[must_use]
    pub const fn diagnostics(&self) -> AssemblerDiagnostics {
        self.diagnostics
    }

    /// Process one byte at its trustworthy wire timestamp.
    ///
    /// A byte at exactly t1.5 remains in the candidate. A byte at exactly t3.5
    /// closes the old candidate and starts a new one. A gap between those
    /// boundaries discards the candidate and quarantines the violating byte as
    /// noise.
    ///
    /// # Errors
    ///
    /// Returns [`AssemblerError`] for timestamp regression or deadline
    /// generation overflow. Apart from diagnostic counters, an error leaves the
    /// state, candidate, and active deadline unchanged.
    pub fn observe_byte(
        &mut self,
        timestamp: RtuTimestamp,
        byte: u8,
    ) -> Result<AssemblerOutcome, AssemblerError> {
        saturating_increment(&mut self.diagnostics.bytes_observed);
        self.check_timestamp(timestamp)?;

        match self.state {
            State::Idle => {
                let (deadline, sequence) = self.plan_deadline(timestamp)?;
                self.start_candidate(timestamp, byte, deadline, sequence);
                Ok(AssemblerOutcome::Progress)
            }
            State::Collecting { len, last_byte, .. } => {
                let gap = timestamp.as_nanos() - last_byte.as_nanos();
                if gap >= self.timing.t3_5_nanos {
                    let (deadline, sequence) = self.plan_deadline(timestamp)?;
                    let outcome = self.finish_candidate(len);
                    self.start_candidate(timestamp, byte, deadline, sequence);
                    return Ok(outcome);
                }
                if gap > self.timing.t1_5_nanos {
                    let (deadline, sequence) = self.plan_deadline(timestamp)?;
                    saturating_increment(&mut self.diagnostics.inter_character_discards);
                    self.enter_quarantine(timestamp, deadline, sequence);
                    return Ok(AssemblerOutcome::Discarded(
                        AssemblerDiscardReason::InterCharacterGap,
                    ));
                }
                if len == MAX_RTU_ADU_SIZE {
                    let (deadline, sequence) = self.plan_deadline(timestamp)?;
                    saturating_increment(&mut self.diagnostics.overlength_discards);
                    self.enter_quarantine(timestamp, deadline, sequence);
                    return Ok(AssemblerOutcome::Discarded(
                        AssemblerDiscardReason::Overlength,
                    ));
                }

                let (deadline, sequence) = self.plan_deadline(timestamp)?;
                self.candidate[len] = byte;
                self.state = State::Collecting {
                    len: len + 1,
                    last_byte: timestamp,
                    deadline,
                };
                self.deadline_sequence = sequence;
                self.last_observed = Some(timestamp);
                Ok(AssemblerOutcome::Progress)
            }
            State::Quarantined { last_noise, .. } => {
                let gap = timestamp.as_nanos() - last_noise.as_nanos();
                let (deadline, sequence) = self.plan_deadline(timestamp)?;
                if gap >= self.timing.t3_5_nanos {
                    self.start_candidate(timestamp, byte, deadline, sequence);
                    saturating_increment(&mut self.diagnostics.quarantine_recoveries);
                    Ok(AssemblerOutcome::Recovered(
                        AssemblerRecovery::CandidateStarted,
                    ))
                } else {
                    self.enter_quarantine(timestamp, deadline, sequence);
                    Ok(AssemblerOutcome::Progress)
                }
            }
        }
    }

    /// Process a callback for a previously returned t3.5 deadline.
    ///
    /// Stale identity is checked before timestamp ordering, so an obsolete
    /// callback is harmless even when `observed_at` predates newer byte events.
    /// An active callback may run at or after its due timestamp.
    ///
    /// # Errors
    ///
    /// Returns [`AssemblerError::TimestampRegression`] for a regressing active
    /// callback or [`AssemblerError::DeadlineNotDue`] when the active callback
    /// runs early. Apart from diagnostic counters, either error is transactional.
    pub fn observe_deadline(
        &mut self,
        deadline: AssemblerDeadline,
        observed_at: RtuTimestamp,
    ) -> Result<AssemblerOutcome, AssemblerError> {
        if self.state.deadline() != Some(deadline) {
            saturating_increment(&mut self.diagnostics.stale_deadlines);
            return Ok(AssemblerOutcome::StaleDeadline);
        }
        self.check_timestamp(observed_at)?;
        if observed_at < deadline.due_at {
            saturating_increment(&mut self.diagnostics.early_deadlines);
            return Err(AssemblerError::DeadlineNotDue {
                deadline,
                observed: observed_at,
            });
        }

        match self.state {
            State::Collecting { len, .. } => {
                let outcome = self.finish_candidate(len);
                self.state = State::Idle;
                self.last_observed = Some(observed_at);
                Ok(outcome)
            }
            State::Quarantined { .. } => {
                self.state = State::Idle;
                self.last_observed = Some(observed_at);
                saturating_increment(&mut self.diagnostics.quarantine_recoveries);
                Ok(AssemblerOutcome::Recovered(AssemblerRecovery::Idle))
            }
            State::Idle => unreachable!("an idle assembler cannot own an active deadline"),
        }
    }

    fn check_timestamp(&mut self, observed: RtuTimestamp) -> Result<(), AssemblerError> {
        if let Some(previous) = self.last_observed
            && observed < previous
        {
            saturating_increment(&mut self.diagnostics.timestamp_regressions);
            return Err(AssemblerError::TimestampRegression { previous, observed });
        }
        Ok(())
    }

    fn plan_deadline(
        &self,
        timestamp: RtuTimestamp,
    ) -> Result<(AssemblerDeadline, u64), AssemblerError> {
        let sequence = self
            .deadline_sequence
            .checked_add(1)
            .ok_or(AssemblerError::DeadlineGenerationOverflow)?;
        let due_at = timestamp
            .checked_add(self.timing.t3_5())
            .ok_or(AssemblerError::TimestampOverflow { timestamp })?;
        Ok((AssemblerDeadline { due_at, sequence }, sequence))
    }

    fn start_candidate(
        &mut self,
        timestamp: RtuTimestamp,
        byte: u8,
        deadline: AssemblerDeadline,
        sequence: u64,
    ) {
        self.candidate[0] = byte;
        self.state = State::Collecting {
            len: 1,
            last_byte: timestamp,
            deadline,
        };
        self.deadline_sequence = sequence;
        self.last_observed = Some(timestamp);
    }

    fn enter_quarantine(
        &mut self,
        timestamp: RtuTimestamp,
        deadline: AssemblerDeadline,
        sequence: u64,
    ) {
        self.state = State::Quarantined {
            last_noise: timestamp,
            deadline,
        };
        self.deadline_sequence = sequence;
        self.last_observed = Some(timestamp);
    }

    fn finish_candidate(&mut self, len: usize) -> AssemblerOutcome {
        if len < MIN_RTU_ADU_SIZE {
            saturating_increment(&mut self.diagnostics.too_short_discards);
            return AssemblerOutcome::Discarded(AssemblerDiscardReason::TooShort);
        }
        if !verify_crc(&self.candidate[..len]) {
            saturating_increment(&mut self.diagnostics.crc_mismatch_discards);
            return AssemblerOutcome::Discarded(AssemblerDiscardReason::CrcMismatch);
        }

        let mut bytes = [0; MAX_RTU_ADU_SIZE];
        bytes[..len].copy_from_slice(&self.candidate[..len]);
        saturating_increment(&mut self.diagnostics.valid_frames);
        AssemblerOutcome::FrameReady(OwnedRtuAdu { bytes, len })
    }
}

fn saturating_increment(counter: &mut u64) {
    *counter = counter.saturating_add(1);
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use rusty_modbus_frame::crc16;

    use super::*;
    use crate::{RtuSerialFormat, StrictRtuConfig};

    const T1_5: u64 = 10;
    const T3_5: u64 = 20;

    fn timing() -> RtuTiming {
        RtuTiming::new(Duration::from_nanos(T1_5), Duration::from_nanos(T3_5)).unwrap()
    }

    fn assembler() -> RtuFrameAssembler {
        RtuFrameAssembler::new(timing())
    }

    fn valid_adu(data: &[u8]) -> Vec<u8> {
        let mut adu = data.to_vec();
        adu.extend_from_slice(&crc16(data).to_le_bytes());
        adu
    }

    fn feed_candidate(
        assembler: &mut RtuFrameAssembler,
        bytes: &[u8],
        start: u64,
        gap: u64,
    ) -> RtuTimestamp {
        let mut timestamp = start;
        for (index, byte) in bytes.iter().copied().enumerate() {
            if index != 0 {
                timestamp += gap;
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

    fn finish_at_active_deadline(assembler: &mut RtuFrameAssembler) -> AssemblerOutcome {
        let deadline = assembler.next_deadline().unwrap();
        assembler
            .observe_deadline(deadline, deadline.due_at())
            .unwrap()
    }

    #[test]
    fn timing_rejects_zero_reversed_and_out_of_range_values() {
        assert_eq!(
            RtuTiming::new(Duration::ZERO, Duration::from_nanos(2)),
            Err(RtuTimingError::InvalidTiming)
        );
        assert_eq!(
            RtuTiming::new(Duration::from_nanos(2), Duration::from_nanos(2)),
            Err(RtuTimingError::InvalidTiming)
        );
        assert_eq!(
            RtuTiming::new(Duration::from_nanos(3), Duration::from_nanos(2)),
            Err(RtuTimingError::InvalidTiming)
        );
        assert_eq!(
            RtuTiming::new(Duration::from_secs(u64::MAX), Duration::from_secs(u64::MAX)),
            Err(RtuTimingError::T1_5OutOfRange)
        );
        assert_eq!(
            RtuTiming::new(Duration::from_nanos(1), Duration::from_secs(u64::MAX)),
            Err(RtuTimingError::T3_5OutOfRange)
        );
    }

    #[test]
    fn timing_converts_directly_from_resolved_config() {
        let resolved = StrictRtuConfig::new(
            19_200,
            RtuSerialFormat::EightEvenOne,
            Duration::from_secs(1),
        )
        .unwrap()
        .resolve();
        let timing = RtuTiming::try_from(&resolved).unwrap();
        let assembler = RtuFrameAssembler::from_resolved(&resolved).unwrap();

        assert_eq!(timing.t1_5(), resolved.t1_5());
        assert_eq!(timing.t3_5(), resolved.t3_5());
        assert_eq!(assembler.timing(), timing);
    }

    #[test]
    fn timestamps_accept_zero_and_use_checked_arithmetic() {
        let zero = RtuTimestamp::from_nanos(0);
        assert_eq!(zero.as_nanos(), 0);
        assert_eq!(
            zero.checked_add(Duration::from_nanos(7)),
            Some(RtuTimestamp::from_nanos(7))
        );
        assert_eq!(
            RtuTimestamp::from_nanos(7).checked_duration_since(zero),
            Some(Duration::from_nanos(7))
        );
        assert_eq!(
            zero.checked_duration_since(RtuTimestamp::from_nanos(1)),
            None
        );
        assert_eq!(
            RtuTimestamp::from_nanos(u64::MAX).checked_add(Duration::from_nanos(1)),
            None
        );
    }

    #[test]
    fn valid_candidate_is_emitted_only_at_t3_5_deadline() {
        let adu = valid_adu(&[1, 3, 0, 0, 0, 1]);
        let mut assembler = assembler();
        feed_candidate(&mut assembler, &adu, 0, 1);

        let outcome = finish_at_active_deadline(&mut assembler);
        let AssemblerOutcome::FrameReady(frame) = outcome else {
            panic!("valid candidate was not emitted: {outcome:?}");
        };
        assert_eq!(frame.as_bytes(), adu);
        assert_eq!(frame.len(), adu.len());
        assert!(!frame.is_empty());
        assert_eq!(assembler.state(), AssemblerState::Idle);
        assert_eq!(assembler.diagnostics().valid_frames, 1);
    }

    #[test]
    fn exactly_t1_5_is_valid_but_just_over_enters_quarantine() {
        let mut at_boundary = assembler();
        assert_eq!(
            at_boundary.observe_byte(RtuTimestamp::from_nanos(0), 1),
            Ok(AssemblerOutcome::Progress)
        );
        assert_eq!(
            at_boundary.observe_byte(RtuTimestamp::from_nanos(T1_5), 3),
            Ok(AssemblerOutcome::Progress)
        );
        assert_eq!(at_boundary.candidate_len(), 2);

        let mut over_boundary = assembler();
        over_boundary
            .observe_byte(RtuTimestamp::from_nanos(0), 1)
            .unwrap();
        assert_eq!(
            over_boundary.observe_byte(RtuTimestamp::from_nanos(T1_5 + 1), 3),
            Ok(AssemblerOutcome::Discarded(
                AssemblerDiscardReason::InterCharacterGap
            ))
        );
        assert_eq!(over_boundary.state(), AssemblerState::Quarantined);
        assert_eq!(over_boundary.candidate_len(), 0);
        assert_eq!(over_boundary.diagnostics().inter_character_discards, 1);
    }

    #[test]
    fn byte_at_t3_5_closes_previous_candidate_and_starts_another() {
        let adu = valid_adu(&[1, 3]);
        let mut assembler = assembler();
        let last = feed_candidate(&mut assembler, &adu, 0, 1);
        let old_deadline = assembler.next_deadline().unwrap();
        let boundary = RtuTimestamp::from_nanos(last.as_nanos() + T3_5);

        let outcome = assembler.observe_byte(boundary, 0xAA).unwrap();
        let AssemblerOutcome::FrameReady(frame) = outcome else {
            panic!("previous candidate was not emitted: {outcome:?}");
        };
        assert_eq!(frame.as_bytes(), adu);
        assert_eq!(assembler.state(), AssemblerState::Collecting);
        assert_eq!(assembler.candidate_len(), 1);
        assert_ne!(assembler.next_deadline(), Some(old_deadline));
        assert_eq!(
            assembler
                .observe_deadline(old_deadline, old_deadline.due_at())
                .unwrap(),
            AssemblerOutcome::StaleDeadline
        );
        assert_eq!(assembler.candidate_len(), 1);
    }

    #[test]
    fn byte_after_t3_5_also_closes_and_restarts() {
        let adu = valid_adu(&[1, 6]);
        let mut assembler = assembler();
        let last = feed_candidate(&mut assembler, &adu, 100, 1);

        assert!(matches!(
            assembler
                .observe_byte(RtuTimestamp::from_nanos(last.as_nanos() + T3_5 + 1), 0x55)
                .unwrap(),
            AssemblerOutcome::FrameReady(_)
        ));
        assert_eq!(assembler.candidate_len(), 1);
    }

    #[test]
    fn short_and_bad_crc_candidates_have_distinct_discards() {
        let mut short = assembler();
        feed_candidate(&mut short, &[1, 3, 0], 0, 1);
        assert_eq!(
            finish_at_active_deadline(&mut short),
            AssemblerOutcome::Discarded(AssemblerDiscardReason::TooShort)
        );

        let mut bad_crc = assembler();
        feed_candidate(&mut bad_crc, &[1, 3, 0, 0], 0, 1);
        assert_eq!(
            finish_at_active_deadline(&mut bad_crc),
            AssemblerOutcome::Discarded(AssemblerDiscardReason::CrcMismatch)
        );
        assert_eq!(short.diagnostics().too_short_discards, 1);
        assert_eq!(bad_crc.diagnostics().crc_mismatch_discards, 1);
    }

    #[test]
    fn crc_valid_prefix_does_not_create_a_hidden_boundary() {
        let mut candidate = valid_adu(&[1, 3]);
        assert!(verify_crc(&candidate));
        candidate.push(0xAA);
        assert!(!verify_crc(&candidate));
        let mut assembler = assembler();
        feed_candidate(&mut assembler, &candidate, 0, 1);

        assert_eq!(
            finish_at_active_deadline(&mut assembler),
            AssemblerOutcome::Discarded(AssemblerDiscardReason::CrcMismatch)
        );
    }

    #[test]
    fn maximum_candidate_is_fixed_and_257th_byte_quarantines() {
        let mut assembler = assembler();
        for timestamp in 0..MAX_RTU_ADU_SIZE as u64 {
            assert_eq!(
                assembler
                    .observe_byte(RtuTimestamp::from_nanos(timestamp), 0xAA)
                    .unwrap(),
                AssemblerOutcome::Progress
            );
        }
        assert_eq!(assembler.candidate_len(), MAX_RTU_ADU_SIZE);
        assert_eq!(
            assembler
                .observe_byte(RtuTimestamp::from_nanos(MAX_RTU_ADU_SIZE as u64), 0xAA)
                .unwrap(),
            AssemblerOutcome::Discarded(AssemblerDiscardReason::Overlength)
        );
        assert_eq!(assembler.state(), AssemblerState::Quarantined);
        assert_eq!(assembler.diagnostics().bytes_observed, 257);
        assert_eq!(assembler.diagnostics().overlength_discards, 1);
    }

    #[test]
    fn full_256_byte_valid_adu_is_emitted_from_inline_storage() {
        let mut data = vec![0x55; MAX_RTU_ADU_SIZE - 2];
        data[0] = 1;
        data[1] = 3;
        let adu = valid_adu(&data);
        assert_eq!(adu.len(), MAX_RTU_ADU_SIZE);
        let mut assembler = assembler();
        feed_candidate(&mut assembler, &adu, 0, 1);

        let AssemblerOutcome::FrameReady(frame) = finish_at_active_deadline(&mut assembler) else {
            panic!("maximum valid ADU was not emitted");
        };
        assert_eq!(frame.as_bytes(), adu);
        assert!(size_of::<OwnedRtuAdu>() < 512);
        assert!(size_of::<RtuFrameAssembler>() < 1024);
    }

    #[test]
    fn quarantine_noise_extends_recovery_boundary() {
        let mut assembler = assembler();
        assembler
            .observe_byte(RtuTimestamp::from_nanos(0), 1)
            .unwrap();
        assembler
            .observe_byte(RtuTimestamp::from_nanos(T1_5 + 1), 2)
            .unwrap();
        let first_recovery = assembler.next_deadline().unwrap();
        let noise_at = T1_5 + 2;
        assert_eq!(
            assembler
                .observe_byte(RtuTimestamp::from_nanos(noise_at), 3)
                .unwrap(),
            AssemblerOutcome::Progress
        );
        let extended = assembler.next_deadline().unwrap();
        assert!(extended.due_at() > first_recovery.due_at());
        assert_eq!(
            assembler
                .observe_deadline(first_recovery, first_recovery.due_at())
                .unwrap(),
            AssemblerOutcome::StaleDeadline
        );
        assert_eq!(
            assembler
                .observe_byte(RtuTimestamp::from_nanos(noise_at + T3_5), 4)
                .unwrap(),
            AssemblerOutcome::Recovered(AssemblerRecovery::CandidateStarted)
        );
        assert_eq!(assembler.state(), AssemblerState::Collecting);
        assert_eq!(assembler.candidate_len(), 1);
    }

    #[test]
    fn active_quarantine_deadline_recovers_to_idle() {
        let mut assembler = assembler();
        assembler
            .observe_byte(RtuTimestamp::from_nanos(0), 1)
            .unwrap();
        assembler
            .observe_byte(RtuTimestamp::from_nanos(T1_5 + 1), 2)
            .unwrap();
        let deadline = assembler.next_deadline().unwrap();

        assert_eq!(
            assembler
                .observe_deadline(deadline, deadline.due_at())
                .unwrap(),
            AssemblerOutcome::Recovered(AssemblerRecovery::Idle)
        );
        assert_eq!(assembler.state(), AssemblerState::Idle);
        assert_eq!(assembler.diagnostics().quarantine_recoveries, 1);
    }

    #[test]
    fn early_deadline_preserves_candidate_until_t3_5() {
        let adu = valid_adu(&[1, 3]);
        let mut assembler = assembler();
        feed_candidate(&mut assembler, &adu, 0, 1);
        let deadline = assembler.next_deadline().unwrap();
        let candidate_len = assembler.candidate_len();

        assert_eq!(
            assembler.observe_deadline(
                deadline,
                RtuTimestamp::from_nanos(deadline.due_at().as_nanos() - 1)
            ),
            Err(AssemblerError::DeadlineNotDue {
                deadline,
                observed: RtuTimestamp::from_nanos(deadline.due_at().as_nanos() - 1),
            })
        );
        assert_eq!(assembler.candidate_len(), candidate_len);
        assert_eq!(assembler.next_deadline(), Some(deadline));
        assert!(matches!(
            finish_at_active_deadline(&mut assembler),
            AssemblerOutcome::FrameReady(_)
        ));
        assert_eq!(assembler.diagnostics().early_deadlines, 1);
    }

    #[test]
    fn stale_deadline_precedes_regression_checks() {
        let mut assembler = assembler();
        assembler
            .observe_byte(RtuTimestamp::from_nanos(100), 1)
            .unwrap();
        let stale = assembler.next_deadline().unwrap();
        assembler
            .observe_byte(RtuTimestamp::from_nanos(101), 3)
            .unwrap();
        let active = assembler.next_deadline();

        assert_eq!(
            assembler
                .observe_deadline(stale, RtuTimestamp::from_nanos(0))
                .unwrap(),
            AssemblerOutcome::StaleDeadline
        );
        assert_eq!(assembler.next_deadline(), active);
        assert_eq!(assembler.diagnostics().stale_deadlines, 1);
        assert_eq!(assembler.diagnostics().timestamp_regressions, 0);
    }

    #[test]
    fn timestamp_regressions_are_transactional_for_bytes_and_active_deadlines() {
        let mut assembler = assembler();
        assembler
            .observe_byte(RtuTimestamp::from_nanos(100), 1)
            .unwrap();
        let deadline = assembler.next_deadline().unwrap();
        let state = assembler.state();
        let len = assembler.candidate_len();

        assert!(matches!(
            assembler.observe_byte(RtuTimestamp::from_nanos(99), 2),
            Err(AssemblerError::TimestampRegression { .. })
        ));
        assert!(matches!(
            assembler.observe_deadline(deadline, RtuTimestamp::from_nanos(99)),
            Err(AssemblerError::TimestampRegression { .. })
        ));
        assert_eq!(assembler.state(), state);
        assert_eq!(assembler.candidate_len(), len);
        assert_eq!(assembler.next_deadline(), Some(deadline));
        assert_eq!(assembler.diagnostics().bytes_observed, 2);
        assert_eq!(assembler.diagnostics().timestamp_regressions, 2);
    }

    #[test]
    fn deadline_arithmetic_overflow_is_transactional_before_append() {
        let mut idle = assembler();
        let overflowing = RtuTimestamp::from_nanos(u64::MAX - T3_5 + 1);
        assert_eq!(
            idle.observe_byte(overflowing, 1),
            Err(AssemblerError::TimestampOverflow {
                timestamp: overflowing
            })
        );
        assert_eq!(idle.state(), AssemblerState::Idle);
        assert_eq!(idle.candidate_len(), 0);
        assert_eq!(idle.next_deadline(), None);

        let mut collecting = assembler();
        let start = RtuTimestamp::from_nanos(u64::MAX - T3_5);
        collecting.observe_byte(start, 1).unwrap();
        let deadline = collecting.next_deadline();
        let next = RtuTimestamp::from_nanos(start.as_nanos() + 1);
        assert_eq!(
            collecting.observe_byte(next, 2),
            Err(AssemblerError::TimestampOverflow { timestamp: next })
        );
        assert_eq!(collecting.candidate_len(), 1);
        assert_eq!(collecting.next_deadline(), deadline);
    }

    #[test]
    fn deadline_generation_overflow_is_transactional() {
        let mut assembler = assembler();
        assembler.deadline_sequence = u64::MAX;

        assert_eq!(
            assembler.observe_byte(RtuTimestamp::from_nanos(0), 1),
            Err(AssemblerError::DeadlineGenerationOverflow)
        );
        assert_eq!(assembler.state(), AssemblerState::Idle);
        assert_eq!(assembler.candidate_len(), 0);
        assert_eq!(assembler.next_deadline(), None);
    }

    #[test]
    fn exact_boundary_deadline_and_byte_orders_reach_same_assembly_state() {
        let adu = valid_adu(&[1, 3]);
        let mut byte_first = assembler();
        let last = feed_candidate(&mut byte_first, &adu, 0, 1);
        let old_deadline = byte_first.next_deadline().unwrap();
        let boundary = RtuTimestamp::from_nanos(last.as_nanos() + T3_5);
        assert_eq!(boundary, old_deadline.due_at());
        assert!(matches!(
            byte_first.observe_byte(boundary, 0xAA).unwrap(),
            AssemblerOutcome::FrameReady(_)
        ));
        assert_eq!(
            byte_first.observe_deadline(old_deadline, boundary).unwrap(),
            AssemblerOutcome::StaleDeadline
        );

        let mut deadline_first = assembler();
        feed_candidate(&mut deadline_first, &adu, 0, 1);
        let deadline = deadline_first.next_deadline().unwrap();
        assert!(matches!(
            deadline_first.observe_deadline(deadline, boundary).unwrap(),
            AssemblerOutcome::FrameReady(_)
        ));
        assert_eq!(
            deadline_first.observe_byte(boundary, 0xAA).unwrap(),
            AssemblerOutcome::Progress
        );

        assert_eq!(byte_first.state(), deadline_first.state());
        assert_eq!(byte_first.candidate_len(), deadline_first.candidate_len());
        assert_eq!(byte_first.next_deadline(), deadline_first.next_deadline());
    }

    #[test]
    fn diagnostics_saturate_under_repeated_observations() {
        let mut assembler = assembler();
        assembler.diagnostics.bytes_observed = u64::MAX;
        assembler.diagnostics.timestamp_regressions = u64::MAX;
        assembler
            .observe_byte(RtuTimestamp::from_nanos(1), 1)
            .unwrap();
        assert!(matches!(
            assembler.observe_byte(RtuTimestamp::from_nanos(0), 2),
            Err(AssemblerError::TimestampRegression { .. })
        ));

        assert_eq!(assembler.diagnostics().bytes_observed, u64::MAX);
        assert_eq!(assembler.diagnostics().timestamp_regressions, u64::MAX);
    }

    #[test]
    fn crc_valid_adu_does_not_require_known_function_semantics() {
        let adu = valid_adu(&[247, 0x00]);
        let mut assembler = assembler();
        feed_candidate(&mut assembler, &adu, 0, 1);

        assert!(matches!(
            finish_at_active_deadline(&mut assembler),
            AssemblerOutcome::FrameReady(_)
        ));
    }
}
