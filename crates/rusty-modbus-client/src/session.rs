//! Local session reuse-safety tracking.

use std::sync::atomic::{AtomicU8, Ordering};

const NOT_QUIESCENT: u8 = 0;
const REUSE_ELIGIBLE: u8 = 1;
const RETIRED_BASE: u8 = 2;

/// A local verdict about whether this client's transport session could be reused safely.
///
/// This verdict describes only ambiguity observed by the client. It is not a peer
/// liveness or health check, and it does not recover, return, or reinsert a transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SessionReuseVerdict {
    /// The client has not completed a clean, quiescent graceful shutdown.
    NotQuiescent,
    /// Graceful shutdown drained all work and joined all client-owned tasks
    /// without observing an ambiguity that requires retirement.
    ReuseEligible,
    /// The session must be retired for the first locally observed reason.
    Retire(SessionRetirementReason),
}

/// The first local condition that made a client session unsafe to reuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SessionRetirementReason {
    /// Immediate client abort was requested.
    Aborted,
    /// The final client owner was dropped without preserving the session.
    FinalOwnerDropped,
    /// Graceful shutdown exceeded its configured drain deadline.
    ShutdownDeadlineExceeded,
    /// Graceful shutdown could not establish a fully drained local state.
    ShutdownIncomplete,
    /// A client-owned background task failed while being joined.
    BackgroundTaskFailed,
    /// A request future was cancelled or dropped after dispatch could have begun.
    DispatchCancelled,
    /// A dispatched request reached a local or transport timeout.
    RequestTimedOut,
    /// The transport reported a request send failure after dispatch could have begun.
    SendFailed,
    /// A response arrived only after its matching request had expired.
    ResponseExpired,
    /// A response carried an unexpected Unit Identifier.
    UnexpectedResponseUnit,
    /// A response carried an unexpected normal or exception function code.
    UnexpectedResponseFunction,
    /// A matching response PDU was malformed and could not be decoded.
    MalformedResponse,
    /// A response had no active matching transaction or duplicated a completed response.
    UnknownOrDuplicateResponse,
    /// A response channel closed without delivering a terminal transaction result.
    ResponseChannelClosed,
    /// The background reader observed transport disconnection.
    ReaderDisconnected,
    /// The background reader observed a non-idle transport error.
    ReaderTransportFailed,
}

impl SessionRetirementReason {
    const fn state(self) -> u8 {
        RETIRED_BASE
            + match self {
                Self::Aborted => 0,
                Self::FinalOwnerDropped => 1,
                Self::ShutdownDeadlineExceeded => 2,
                Self::ShutdownIncomplete => 3,
                Self::BackgroundTaskFailed => 4,
                Self::DispatchCancelled => 5,
                Self::RequestTimedOut => 6,
                Self::SendFailed => 7,
                Self::ResponseExpired => 8,
                Self::UnexpectedResponseUnit => 9,
                Self::UnexpectedResponseFunction => 10,
                Self::MalformedResponse => 11,
                Self::UnknownOrDuplicateResponse => 12,
                Self::ResponseChannelClosed => 13,
                Self::ReaderDisconnected => 14,
                Self::ReaderTransportFailed => 15,
            }
    }

    fn from_state(state: u8) -> Self {
        match state - RETIRED_BASE {
            0 => Self::Aborted,
            1 => Self::FinalOwnerDropped,
            2 => Self::ShutdownDeadlineExceeded,
            3 => Self::ShutdownIncomplete,
            4 => Self::BackgroundTaskFailed,
            5 => Self::DispatchCancelled,
            6 => Self::RequestTimedOut,
            7 => Self::SendFailed,
            8 => Self::ResponseExpired,
            9 => Self::UnexpectedResponseUnit,
            10 => Self::UnexpectedResponseFunction,
            11 => Self::MalformedResponse,
            12 => Self::UnknownOrDuplicateResponse,
            13 => Self::ResponseChannelClosed,
            14 => Self::ReaderDisconnected,
            15 => Self::ReaderTransportFailed,
            _ => unreachable!("reuse-safety authority stored an invalid state"),
        }
    }
}

/// One-way, first-reason-wins authority shared by all client-owned paths.
pub(crate) struct SessionReuseSafety {
    state: AtomicU8,
}

impl SessionReuseSafety {
    pub(crate) const fn new() -> Self {
        Self {
            state: AtomicU8::new(NOT_QUIESCENT),
        }
    }

    pub(crate) fn verdict(&self) -> SessionReuseVerdict {
        match self.state.load(Ordering::Acquire) {
            NOT_QUIESCENT => SessionReuseVerdict::NotQuiescent,
            REUSE_ELIGIBLE => SessionReuseVerdict::ReuseEligible,
            retired => SessionReuseVerdict::Retire(SessionRetirementReason::from_state(retired)),
        }
    }

    pub(crate) fn retire(&self, reason: SessionRetirementReason) {
        let retired = reason.state();
        let mut current = self.state.load(Ordering::Acquire);
        loop {
            if current >= RETIRED_BASE {
                return;
            }
            match self.state.compare_exchange_weak(
                current,
                retired,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(observed) => current = observed,
            }
        }
    }

    pub(crate) fn mark_clean_shutdown(&self) {
        let _ = self.state.compare_exchange(
            NOT_QUIESCENT,
            REUSE_ELIGIBLE,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

/// Borrowed stack guard for the interval where dispatch may be ambiguous.
pub(crate) struct DispatchGuard<'a> {
    reuse_safety: &'a SessionReuseSafety,
    armed: bool,
}

impl<'a> DispatchGuard<'a> {
    pub(crate) const fn armed(reuse_safety: &'a SessionReuseSafety) -> Self {
        Self {
            reuse_safety,
            armed: true,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for DispatchGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.reuse_safety
                .retire(SessionRetirementReason::DispatchCancelled);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retirement_is_sticky_and_first_reason_wins() {
        let safety = SessionReuseSafety::new();
        safety.mark_clean_shutdown();
        assert_eq!(safety.verdict(), SessionReuseVerdict::ReuseEligible);

        safety.retire(SessionRetirementReason::Aborted);
        safety.retire(SessionRetirementReason::ReaderDisconnected);
        safety.mark_clean_shutdown();

        assert_eq!(
            safety.verdict(),
            SessionReuseVerdict::Retire(SessionRetirementReason::Aborted)
        );
    }
}
