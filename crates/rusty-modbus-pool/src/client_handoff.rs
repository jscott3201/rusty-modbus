//! Pool-owned adapters for handing a raw TCP lease to the client engine.

use std::net::SocketAddr;
use std::sync::Arc;

use parking_lot::Mutex;
use rusty_modbus_client::{
    ClientConfig, ModbusClient, SessionRetirementReason, SessionReuseVerdict,
};
use rusty_modbus_frame::Frame;
use rusty_modbus_tcp::{
    TcpRecvStream, TcpSink, TransportError,
    transport::{TransportSink, TransportStream},
};
use tokio::sync::Notify;
use tokio::time::Instant;

use crate::pool::{PoolEntry, PoolInner};

/// Releases one active pool charge when both transport adapters are gone.
struct RetirementGuard {
    pool: Arc<Mutex<PoolInner>>,
    capacity_changed: Arc<Notify>,
    addr: SocketAddr,
    is_priority: bool,
}

impl Drop for RetirementGuard {
    fn drop(&mut self) {
        {
            let mut inner = self.pool.lock();
            inner.retire_active(self.is_priority, self.addr);
        }
        self.capacity_changed.notify_waiters();
    }
}

/// Client-owned write half that keeps the pool charge until it is dropped.
pub(crate) struct RetiringSink {
    // Field order is significant: retire the transport before releasing the
    // guard's final shared reference.
    inner: TcpSink,
    _retirement: Arc<RetirementGuard>,
}

impl TransportSink for RetiringSink {
    async fn send(&mut self, frame: Frame) -> Result<(), TransportError> {
        self.inner.send(frame).await
    }
}

/// Client-owned read half that keeps the pool charge until it is dropped.
struct RetiringStream {
    // See `RetiringSink`: the TCP half must be dropped before the shared guard.
    inner: TcpRecvStream,
    _retirement: Arc<RetirementGuard>,
}

impl TransportStream for RetiringStream {
    async fn recv(&mut self) -> Result<Frame, TransportError> {
        self.inner.recv().await
    }
}

pub(crate) fn into_client(
    entry: PoolEntry,
    pool: Arc<Mutex<PoolInner>>,
    capacity_changed: Arc<Notify>,
    config: ClientConfig,
) -> ModbusClient<RetiringSink> {
    let retirement = Arc::new(RetirementGuard {
        pool,
        capacity_changed,
        addr: entry.addr,
        is_priority: entry.is_priority,
    });
    let PoolEntry { sink, stream, .. } = entry;

    let sink = RetiringSink {
        inner: sink,
        _retirement: Arc::clone(&retirement),
    };
    let stream = RetiringStream {
        inner: stream,
        _retirement: retirement,
    };

    ModbusClient::from_transport(sink, stream, config)
}

/// Outcome of consuming a reusable pooled client session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PooledClientReturnOutcome {
    /// Graceful shutdown was locally clean and the TCP connection returned to idle.
    ReturnedToIdle,
    /// The final local verdict was not eligible for reuse, so the connection retired.
    Retired(SessionReuseVerdict),
    /// Pool shutdown won the return race, so the connection retired.
    PoolShuttingDown,
    /// The private transport vault could not recover a complete lease.
    ///
    /// This indicates an internal handoff-state failure, not a client reuse verdict.
    TransportRecoveryFailed,
}

#[derive(Clone, Copy)]
enum SessionCompletionTrigger {
    ShutdownAndReturn,
    WrapperDrop,
}

impl SessionCompletionTrigger {
    const fn label(self) -> &'static str {
        match self {
            Self::ShutdownAndReturn => "shutdown_and_return",
            Self::WrapperDrop => "wrapper_drop",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct SessionCompletionFields {
    outcome: &'static str,
    trigger: &'static str,
    verdict: &'static str,
    retirement_reason: &'static str,
    is_priority: bool,
}

fn session_completion_fields(
    outcome: PooledClientReturnOutcome,
    trigger: SessionCompletionTrigger,
    verdict: SessionReuseVerdict,
    is_priority: bool,
) -> SessionCompletionFields {
    let outcome = pooled_client_outcome_label(outcome);
    let (verdict, retirement_reason) = session_reuse_verdict_labels(verdict);

    SessionCompletionFields {
        outcome,
        trigger: trigger.label(),
        verdict,
        retirement_reason,
        is_priority,
    }
}

pub(crate) const fn pooled_client_outcome_label(
    outcome: PooledClientReturnOutcome,
) -> &'static str {
    match outcome {
        PooledClientReturnOutcome::ReturnedToIdle => "returned_to_idle",
        PooledClientReturnOutcome::Retired(_) => "retired",
        PooledClientReturnOutcome::PoolShuttingDown => "pool_shutting_down",
        PooledClientReturnOutcome::TransportRecoveryFailed => "transport_recovery_failed",
    }
}

pub(crate) const fn session_reuse_verdict_labels(
    verdict: SessionReuseVerdict,
) -> (&'static str, &'static str) {
    match verdict {
        SessionReuseVerdict::ReuseEligible => ("reuse_eligible", "none"),
        SessionReuseVerdict::NotQuiescent => ("not_quiescent", "none"),
        SessionReuseVerdict::Retire(reason) => ("retire", retirement_reason_label(reason)),
        // Future verdicts conservatively retire in `shutdown_and_return`; keep
        // their observability bounded without claiming a current reason.
        _ => ("retire", "other"),
    }
}

pub(crate) const fn retirement_reason_label(reason: SessionRetirementReason) -> &'static str {
    match reason {
        SessionRetirementReason::Aborted => "aborted",
        SessionRetirementReason::FinalOwnerDropped => "final_owner_dropped",
        SessionRetirementReason::ShutdownDeadlineExceeded => "shutdown_deadline_exceeded",
        SessionRetirementReason::ShutdownIncomplete => "shutdown_incomplete",
        SessionRetirementReason::BackgroundTaskFailed => "background_task_failed",
        SessionRetirementReason::DispatchCancelled => "dispatch_cancelled",
        SessionRetirementReason::RequestTimedOut => "request_timed_out",
        SessionRetirementReason::SendFailed => "send_failed",
        SessionRetirementReason::ResponseExpired => "response_expired",
        SessionRetirementReason::UnexpectedResponseUnit => "unexpected_response_unit",
        SessionRetirementReason::UnexpectedResponseFunction => "unexpected_response_function",
        SessionRetirementReason::MalformedResponse => "malformed_response",
        SessionRetirementReason::UnknownOrDuplicateResponse => "unknown_or_duplicate_response",
        SessionRetirementReason::ResponseChannelClosed => "response_channel_closed",
        SessionRetirementReason::ReaderDisconnected => "reader_disconnected",
        SessionRetirementReason::ReaderTransportFailed => "reader_transport_failed",
        SessionRetirementReason::TypedResponseDataInvalid => "typed_response_data_invalid",
        SessionRetirementReason::TypedResponseEchoMismatch => "typed_response_echo_mismatch",
        _ => "other",
    }
}

fn emit_session_completion(
    outcome: PooledClientReturnOutcome,
    trigger: SessionCompletionTrigger,
    verdict: SessionReuseVerdict,
    is_priority: bool,
) {
    let fields = session_completion_fields(outcome, trigger, verdict, is_priority);
    if matches!(outcome, PooledClientReturnOutcome::TransportRecoveryFailed) {
        tracing::warn!(
            target: "rusty_modbus_pool::client_handoff",
            outcome = fields.outcome,
            trigger = fields.trigger,
            verdict = fields.verdict,
            retirement_reason = fields.retirement_reason,
            is_priority = fields.is_priority,
            "pooled_client_session_completed"
        );
    } else {
        tracing::debug!(
            target: "rusty_modbus_pool::client_handoff",
            outcome = fields.outcome,
            trigger = fields.trigger,
            verdict = fields.verdict,
            retirement_reason = fields.retirement_reason,
            is_priority = fields.is_priority,
            "pooled_client_session_completed"
        );
    }
}

/// An opt-in high-level client session that can return a locally clean TCP lease.
///
/// Use [`client`](Self::client) for normal high-level client operations. Only
/// [`shutdown_and_return`](Self::shutdown_and_return) can return the connection
/// to the idle pool. Dropping this wrapper, aborting its client, cancelling work,
/// or observing any verdict other than [`SessionReuseVerdict::ReuseEligible`]
/// retires the transport instead.
///
/// Eligibility is a local synchronization-safety result. It assumes a conforming
/// peer does not invent a future duplicate after all valid requests complete; it
/// is not an active probe or proof of peer liveness or permanent future silence.
/// It does not close the tracked F-017/F-018 recovery gaps.
pub struct PooledClientSession {
    // Drop the client first so its final-owner cancellation reaches any task
    // still holding an adapter before this direct vault reference is released.
    client: ModbusClient<ReusableSink>,
    vault: Arc<ReusableVault>,
    is_priority: bool,
    completion_recorded: bool,
}

impl PooledClientSession {
    /// Borrow the high-level client without exposing the private TCP adapter type.
    #[must_use]
    pub fn client(&self) -> &ModbusClient<impl TransportSink + 'static> {
        &self.client
    }

    /// Gracefully shut down the client and conditionally return its TCP lease.
    ///
    /// Both raw transport halves are recovered only after graceful shutdown has
    /// joined all client-owned tasks and the final local verdict is exactly
    /// [`SessionReuseVerdict::ReuseEligible`]. Pool shutdown is checked atomically
    /// with active-capacity release and idle insertion. Every other path retires
    /// the transport and releases its capacity charge exactly once.
    pub async fn shutdown_and_return(mut self) -> PooledClientReturnOutcome {
        self.client.shutdown().await;
        let verdict = self.client.session_reuse_verdict();
        let outcome = if verdict != SessionReuseVerdict::ReuseEligible {
            PooledClientReturnOutcome::Retired(verdict)
        } else if let Some(recovered) = self.vault.recover().await {
            recovered.return_to_pool()
        } else {
            PooledClientReturnOutcome::TransportRecoveryFailed
        };

        self.completion_recorded = true;
        emit_session_completion(
            outcome,
            SessionCompletionTrigger::ShutdownAndReturn,
            verdict,
            self.is_priority,
        );
        outcome
    }
}

impl Drop for PooledClientSession {
    fn drop(&mut self) {
        if self.completion_recorded {
            return;
        }

        self.completion_recorded = true;
        let verdict = self.client.session_reuse_verdict();
        emit_session_completion(
            PooledClientReturnOutcome::Retired(verdict),
            SessionCompletionTrigger::WrapperDrop,
            verdict,
            self.is_priority,
        );
    }
}

impl std::fmt::Debug for PooledClientSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PooledClientSession")
            .field("client", &self.client)
            .finish_non_exhaustive()
    }
}

/// One exact active-capacity charge owned by a reusable handoff.
struct ActiveCapacityGuard {
    pool: Arc<Mutex<PoolInner>>,
    capacity_changed: Arc<Notify>,
    addr: SocketAddr,
    is_priority: bool,
    armed: bool,
}

impl ActiveCapacityGuard {
    fn charge_exists(&self, inner: &PoolInner) -> bool {
        if self.is_priority {
            inner
                .active_priority
                .get(&self.addr)
                .is_some_and(|count| *count > 0)
        } else {
            inner.active_non_priority > 0
        }
    }
}

impl Drop for ActiveCapacityGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        let pool = Arc::clone(&self.pool);
        {
            let mut inner = pool.lock();
            debug_assert!(
                self.charge_exists(&inner),
                "reusable handoff lost its active capacity charge"
            );
            self.armed = false;
            inner.retire_active(self.is_priority, self.addr);
        }
        self.capacity_changed.notify_waiters();
    }
}

/// Shared raw-half vault. Explicit drop ordering retires both halves before the
/// final active-capacity guard on every non-return path.
struct ReusableVault {
    sink: tokio::sync::Mutex<Option<TcpSink>>,
    stream: tokio::sync::Mutex<Option<TcpRecvStream>>,
    capacity: Mutex<Option<ActiveCapacityGuard>>,
}

impl ReusableVault {
    async fn recover(&self) -> Option<RecoveredLease> {
        // Shutdown has joined the reader and drained active operations, so no
        // adapter operation can contend here. Lock both halves before taking
        // either one so cancellation cannot leave a partially extracted lease.
        let mut sink = self.sink.lock().await;
        let mut stream = self.stream.lock().await;
        let mut capacity = self.capacity.lock();

        if sink.is_none() || stream.is_none() || capacity.is_none() {
            return None;
        }

        let recovered = (sink.take(), stream.take(), capacity.take());
        match recovered {
            (Some(sink), Some(stream), Some(capacity)) => Some(RecoveredLease {
                entry: Some(PoolEntry {
                    addr: capacity.addr,
                    sink,
                    stream,
                    last_used: Instant::now(),
                    is_priority: capacity.is_priority,
                }),
                capacity: Some(capacity),
            }),
            (recovered_sink, recovered_stream, recovered_capacity) => {
                // Keep retirement ownership intact even if an impossible
                // partially populated internal state is encountered.
                *sink = recovered_sink;
                *stream = recovered_stream;
                *capacity = recovered_capacity;
                None
            }
        }
    }
}

impl Drop for ReusableVault {
    fn drop(&mut self) {
        let sink = self.sink.get_mut().take();
        let stream = self.stream.get_mut().take();
        drop(sink);
        drop(stream);

        let capacity = self.capacity.get_mut().take();
        drop(capacity);
    }
}

/// Complete recovered lease with explicit transport-before-capacity drop order.
struct RecoveredLease {
    entry: Option<PoolEntry>,
    capacity: Option<ActiveCapacityGuard>,
}

impl RecoveredLease {
    fn return_to_pool(mut self) -> PooledClientReturnOutcome {
        let (Some(entry), Some(capacity)) = (self.entry.as_ref(), self.capacity.as_ref()) else {
            return PooledClientReturnOutcome::TransportRecoveryFailed;
        };
        debug_assert_eq!(entry.addr, capacity.addr);
        debug_assert_eq!(entry.is_priority, capacity.is_priority);

        let pool = Arc::clone(&capacity.pool);
        let capacity_changed = Arc::clone(&capacity.capacity_changed);
        let mut inner = pool.lock();
        debug_assert!(
            capacity.charge_exists(&inner),
            "reusable handoff lost its active capacity charge"
        );

        // Disarm immediately before the one synchronous accounting release.
        // No fallible operation occurs between these two statements.
        if let Some(capacity) = self.capacity.as_mut() {
            capacity.armed = false;
            if inner.shutting_down {
                inner.retire_active(capacity.is_priority, capacity.addr);
            } else {
                inner.release_active(capacity.is_priority, capacity.addr);
            }
        }

        let shutting_down = inner.shutting_down;
        if !shutting_down && let Some(mut entry) = self.entry.take() {
            entry.last_used = Instant::now();
            inner.idle.push(entry);
        }
        drop(inner);

        if shutting_down {
            // Retire the recovered halves outside the pool lock.
            drop(self.entry.take());
        }
        capacity_changed.notify_waiters();

        if shutting_down {
            PooledClientReturnOutcome::PoolShuttingDown
        } else {
            PooledClientReturnOutcome::ReturnedToIdle
        }
    }
}

impl Drop for RecoveredLease {
    fn drop(&mut self) {
        drop(self.entry.take());
        drop(self.capacity.take());
    }
}

/// Client write adapter borrowing the pool-owned raw sink from the vault.
struct ReusableSink {
    vault: Arc<ReusableVault>,
}

impl TransportSink for ReusableSink {
    async fn send(&mut self, frame: Frame) -> Result<(), TransportError> {
        let mut sink = self.vault.sink.lock().await;
        let Some(sink) = sink.as_mut() else {
            return Err(TransportError::Disconnected);
        };
        sink.send(frame).await
    }
}

/// Client read adapter borrowing the pool-owned raw stream from the vault.
struct ReusableStream {
    vault: Arc<ReusableVault>,
}

impl TransportStream for ReusableStream {
    async fn recv(&mut self) -> Result<Frame, TransportError> {
        let mut stream = self.vault.stream.lock().await;
        let Some(stream) = stream.as_mut() else {
            return Err(TransportError::Disconnected);
        };
        stream.recv().await
    }
}

pub(crate) fn into_reusable_session(
    entry: PoolEntry,
    pool: Arc<Mutex<PoolInner>>,
    capacity_changed: Arc<Notify>,
    config: ClientConfig,
) -> PooledClientSession {
    let PoolEntry {
        addr,
        sink,
        stream,
        is_priority,
        ..
    } = entry;
    let vault = Arc::new(ReusableVault {
        sink: tokio::sync::Mutex::new(Some(sink)),
        stream: tokio::sync::Mutex::new(Some(stream)),
        capacity: Mutex::new(Some(ActiveCapacityGuard {
            pool,
            capacity_changed,
            addr,
            is_priority,
            armed: true,
        })),
    });
    let client = ModbusClient::from_transport(
        ReusableSink {
            vault: Arc::clone(&vault),
        },
        ReusableStream {
            vault: Arc::clone(&vault),
        },
        config,
    );

    PooledClientSession {
        client,
        vault,
        is_priority,
        completion_recorded: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impossible_recovered_lease_maps_transport_recovery_failure_fields() {
        let outcome = RecoveredLease {
            entry: None,
            capacity: None,
        }
        .return_to_pool();
        assert_eq!(outcome, PooledClientReturnOutcome::TransportRecoveryFailed);
        assert_eq!(
            session_completion_fields(
                outcome,
                SessionCompletionTrigger::ShutdownAndReturn,
                SessionReuseVerdict::ReuseEligible,
                false,
            ),
            SessionCompletionFields {
                outcome: "transport_recovery_failed",
                trigger: "shutdown_and_return",
                verdict: "reuse_eligible",
                retirement_reason: "none",
                is_priority: false,
            }
        );
    }

    #[test]
    fn current_retirement_reasons_have_stable_bounded_labels() {
        let cases = [
            (SessionRetirementReason::Aborted, "aborted"),
            (
                SessionRetirementReason::FinalOwnerDropped,
                "final_owner_dropped",
            ),
            (
                SessionRetirementReason::ShutdownDeadlineExceeded,
                "shutdown_deadline_exceeded",
            ),
            (
                SessionRetirementReason::ShutdownIncomplete,
                "shutdown_incomplete",
            ),
            (
                SessionRetirementReason::BackgroundTaskFailed,
                "background_task_failed",
            ),
            (
                SessionRetirementReason::DispatchCancelled,
                "dispatch_cancelled",
            ),
            (
                SessionRetirementReason::RequestTimedOut,
                "request_timed_out",
            ),
            (SessionRetirementReason::SendFailed, "send_failed"),
            (SessionRetirementReason::ResponseExpired, "response_expired"),
            (
                SessionRetirementReason::UnexpectedResponseUnit,
                "unexpected_response_unit",
            ),
            (
                SessionRetirementReason::UnexpectedResponseFunction,
                "unexpected_response_function",
            ),
            (
                SessionRetirementReason::MalformedResponse,
                "malformed_response",
            ),
            (
                SessionRetirementReason::UnknownOrDuplicateResponse,
                "unknown_or_duplicate_response",
            ),
            (
                SessionRetirementReason::ResponseChannelClosed,
                "response_channel_closed",
            ),
            (
                SessionRetirementReason::ReaderDisconnected,
                "reader_disconnected",
            ),
            (
                SessionRetirementReason::ReaderTransportFailed,
                "reader_transport_failed",
            ),
            (
                SessionRetirementReason::TypedResponseDataInvalid,
                "typed_response_data_invalid",
            ),
            (
                SessionRetirementReason::TypedResponseEchoMismatch,
                "typed_response_echo_mismatch",
            ),
        ];

        for (reason, expected) in cases {
            assert_eq!(retirement_reason_label(reason), expected);
        }
    }
}
