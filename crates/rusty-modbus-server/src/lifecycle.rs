//! Server admission, metrics, and owned-task shutdown coordination.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use rusty_modbus_tcp::TcpServerMetrics;
use tokio::sync::watch;
use tokio::task::{AbortHandle, JoinHandle};
use tokio::time::Instant;
use tracing::warn;

fn saturating_instant_add(start: Instant, duration: Duration) -> Instant {
    if let Some(deadline) = start.checked_add(duration) {
        return deadline;
    }

    let mut lower = Duration::ZERO;
    let mut upper = duration;
    while upper.saturating_sub(lower) > Duration::from_nanos(1) {
        let midpoint = lower.saturating_add(upper.saturating_sub(lower) / 2);
        if start.checked_add(midpoint).is_some() {
            lower = midpoint;
        } else {
            upper = midpoint;
        }
    }
    start.checked_add(lower).unwrap_or(start)
}

/// Result of a completed graceful-stop attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownOutcome {
    /// Every admitted connection finished before the configured deadline.
    Drained,
    /// The deadline elapsed and remaining connection tasks were aborted and joined.
    Forced,
}

impl fmt::Display for ShutdownOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Drained => f.write_str("drained"),
            Self::Forced => f.write_str("forced"),
        }
    }
}

/// Point-in-time server counters.
///
/// Snapshots are immutable but not transactional: counters may advance while a
/// snapshot is being collected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ServerMetrics {
    /// Connections currently holding a TCP listener admission reservation.
    pub active_connections: usize,
    /// Requests currently executing a handler or sending its response.
    pub active_requests: usize,
    /// Connections accepted by the TCP listener.
    pub accepted_connections: usize,
    /// Connections rejected by IP access control.
    pub access_denied_connections: usize,
    /// Connections rejected because all admission slots were occupied.
    pub connection_limit_rejections: usize,
    /// Listener accept operations that returned an error.
    pub accept_errors: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct ServerMetricsHandle {
    tcp: TcpServerMetrics,
    active_requests: Arc<AtomicUsize>,
    accept_errors: Arc<AtomicUsize>,
}

impl ServerMetricsHandle {
    fn new(tcp: TcpServerMetrics) -> Self {
        Self {
            tcp,
            active_requests: Arc::new(AtomicUsize::new(0)),
            accept_errors: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(crate) fn snapshot(&self) -> ServerMetrics {
        let tcp = self.tcp.snapshot();
        ServerMetrics {
            active_connections: tcp.active_connections,
            active_requests: self.active_requests.load(Ordering::Relaxed),
            accepted_connections: tcp.accepted_connections,
            access_denied_connections: tcp.access_denied_connections,
            connection_limit_rejections: tcp.connection_limit_rejections,
            accept_errors: self.accept_errors.load(Ordering::Relaxed),
        }
    }

    fn begin_request(&self) -> RequestGuard {
        self.active_requests.fetch_add(1, Ordering::Relaxed);
        RequestGuard {
            active_requests: Arc::clone(&self.active_requests),
        }
    }

    pub(crate) fn record_accept_error(&self) {
        let _ = self
            .accept_errors
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_add(1))
            });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Running,
    Draining,
    Closed,
}

#[derive(Debug)]
struct State {
    phase: Phase,
    deadline: Option<Instant>,
    outcome: Option<ShutdownOutcome>,
    coordinator_started: bool,
}

#[derive(Default)]
struct Tasks {
    supervisor: Option<JoinHandle<ShutdownOutcome>>,
    supervisor_abort: Option<AbortHandle>,
    coordinator_abort: Option<AbortHandle>,
}

pub(crate) struct ServerLifecycle {
    state: Mutex<State>,
    shutdown_tx: watch::Sender<Option<Instant>>,
    completion_tx: watch::Sender<Option<ShutdownOutcome>>,
    metrics: ServerMetricsHandle,
    tasks: Mutex<Tasks>,
}

impl ServerLifecycle {
    pub(crate) fn new(tcp_metrics: TcpServerMetrics) -> Arc<Self> {
        let (shutdown_tx, _) = watch::channel(None);
        let (completion_tx, _) = watch::channel(None);
        Arc::new(Self {
            state: Mutex::new(State {
                phase: Phase::Running,
                deadline: None,
                outcome: None,
                coordinator_started: false,
            }),
            shutdown_tx,
            completion_tx,
            metrics: ServerMetricsHandle::new(tcp_metrics),
            tasks: Mutex::new(Tasks::default()),
        })
    }

    pub(crate) fn shutdown_receiver(&self) -> watch::Receiver<Option<Instant>> {
        self.shutdown_tx.subscribe()
    }

    pub(crate) fn install_supervisor(&self, supervisor: JoinHandle<ShutdownOutcome>) {
        let mut tasks = self.tasks.lock();
        tasks.supervisor_abort = Some(supervisor.abort_handle());
        tasks.supervisor = Some(supervisor);
    }

    pub(crate) fn metrics_handle(&self) -> ServerMetricsHandle {
        self.metrics.clone()
    }

    pub(crate) fn metrics(&self) -> ServerMetrics {
        self.metrics.snapshot()
    }

    /// The state lock is the linearization point shared with `seal`.
    pub(crate) fn shutdown_deadline(&self) -> Option<Instant> {
        self.state.lock().deadline
    }

    pub(crate) fn admit_request(&self) -> Option<RequestGuard> {
        let state = self.state.lock();
        (state.phase == Phase::Running).then(|| self.metrics.begin_request())
    }

    pub(crate) async fn shutdown(self: &Arc<Self>, timeout: Duration) -> ShutdownOutcome {
        self.seal(timeout);
        self.ensure_coordinator();
        self.wait_for_outcome().await
    }

    fn seal(&self, timeout: Duration) {
        let deadline = {
            let mut state = self.state.lock();
            if state.phase == Phase::Running {
                let deadline = saturating_instant_add(Instant::now(), timeout);
                state.phase = Phase::Draining;
                state.deadline = Some(deadline);
                deadline
            } else {
                state
                    .deadline
                    .expect("a stopped server must retain its shutdown deadline")
            }
        };
        self.shutdown_tx.send_replace(Some(deadline));
    }

    fn ensure_coordinator(self: &Arc<Self>) {
        let should_start = {
            let mut state = self.state.lock();
            if state.phase == Phase::Closed || state.coordinator_started {
                false
            } else {
                state.coordinator_started = true;
                true
            }
        };
        if !should_start {
            return;
        }

        let supervisor = self
            .tasks
            .lock()
            .supervisor
            .take()
            .expect("server supervisor must be installed before start returns");
        let lifecycle = Arc::clone(self);
        let coordinator = tokio::spawn(async move {
            let outcome = match supervisor.await {
                Ok(outcome) => outcome,
                Err(error) => {
                    warn!(%error, "Modbus server supervisor failed during shutdown");
                    ShutdownOutcome::Forced
                }
            };
            {
                let mut state = lifecycle.state.lock();
                state.phase = Phase::Closed;
                state.outcome = Some(outcome);
            }
            lifecycle.completion_tx.send_replace(Some(outcome));
        });
        self.tasks.lock().coordinator_abort = Some(coordinator.abort_handle());
        drop(coordinator);
    }

    async fn wait_for_outcome(&self) -> ShutdownOutcome {
        let mut completion_rx = self.completion_tx.subscribe();
        loop {
            if let Some(outcome) = self.state.lock().outcome {
                return outcome;
            }
            completion_rx
                .changed()
                .await
                .expect("server lifecycle retains the completion sender");
        }
    }

    pub(crate) fn abort_owned_tasks(&self) {
        let tasks = self.tasks.lock();
        if let Some(abort) = &tasks.supervisor_abort {
            abort.abort();
        }
        if let Some(abort) = &tasks.coordinator_abort {
            abort.abort();
        }
    }
}

pub(crate) struct RequestGuard {
    active_requests: Arc<AtomicUsize>,
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        let previous = self.active_requests.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(previous > 0, "active request count underflow");
    }
}
