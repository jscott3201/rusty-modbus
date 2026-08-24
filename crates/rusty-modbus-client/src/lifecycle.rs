//! Client-owned admission, cancellation, and task lifecycle.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, watch};
use tokio::task::{AbortHandle, JoinHandle};
use tokio::time::Instant;
use tracing::{debug, warn};

use crate::error::ClientError;
use crate::transaction::TransactionManager;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Running,
    Draining,
    Cancelling,
    Closed,
}

struct State {
    phase: Phase,
    active: usize,
    drain_deadline: Option<Instant>,
    coordinator_started: bool,
}

#[derive(Default)]
struct Tasks {
    reader: Option<JoinHandle<()>>,
    deadline: Option<JoinHandle<()>>,
    reader_abort: Option<AbortHandle>,
    deadline_abort: Option<AbortHandle>,
    coordinator_abort: Option<AbortHandle>,
}

/// Shared authority for client admission and one-way shutdown.
pub(crate) struct ClientLifecycle {
    state: Mutex<State>,
    semaphore: Arc<Semaphore>,
    active_changed: Notify,
    cancellation_tx: watch::Sender<bool>,
    task_stop_tx: watch::Sender<bool>,
    completion_tx: watch::Sender<bool>,
    tasks: Mutex<Tasks>,
    txn_mgr: Arc<TransactionManager>,
    connected: Arc<AtomicBool>,
}

impl ClientLifecycle {
    /// Create a running lifecycle with the configured admission capacity.
    pub(crate) fn new(
        max_in_flight: usize,
        txn_mgr: Arc<TransactionManager>,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        let (cancellation_tx, _) = watch::channel(false);
        let (task_stop_tx, _) = watch::channel(false);
        let (completion_tx, _) = watch::channel(false);
        Arc::new(Self {
            state: Mutex::new(State {
                phase: Phase::Running,
                active: 0,
                drain_deadline: None,
                coordinator_started: false,
            }),
            semaphore: Arc::new(Semaphore::new(max_in_flight)),
            active_changed: Notify::new(),
            cancellation_tx,
            task_stop_tx,
            completion_tx,
            tasks: Mutex::new(Tasks::default()),
            txn_mgr,
            connected,
        })
    }

    /// Subscribe to the durable task-stop signal.
    pub(crate) fn task_stop_receiver(&self) -> watch::Receiver<bool> {
        self.task_stop_tx.subscribe()
    }

    /// Install the client-owned task handles after spawning them.
    pub(crate) fn install_tasks(&self, reader: JoinHandle<()>, deadline: JoinHandle<()>) {
        let mut tasks = self.tasks.lock();
        tasks.reader_abort = Some(reader.abort_handle());
        tasks.deadline_abort = Some(deadline.abort_handle());
        tasks.reader = Some(reader);
        tasks.deadline = Some(deadline);
    }

    /// Whether new operations may be admitted.
    pub(crate) fn is_running(&self) -> bool {
        self.state.lock().phase == Phase::Running
    }

    /// Current lifecycle phase for diagnostics.
    pub(crate) fn phase_name(&self) -> &'static str {
        match self.state.lock().phase {
            Phase::Running => "running",
            Phase::Draining => "draining",
            Phase::Cancelling => "cancelling",
            Phase::Closed => "closed",
        }
    }

    /// Number of admitted logical operations.
    pub(crate) fn active_count(&self) -> usize {
        self.state.lock().active
    }

    /// Acquire and register one logical operation.
    pub(crate) async fn admit(self: &Arc<Self>) -> Result<OperationGuard, ClientError> {
        {
            let state = self.state.lock();
            if state.phase != Phase::Running {
                return Err(ClientError::NotConnected);
            }
        }

        let permit = Arc::clone(&self.semaphore)
            .acquire_owned()
            .await
            .map_err(|_| ClientError::ShuttingDown)?;

        let mut state = self.state.lock();
        if state.phase != Phase::Running {
            return Err(ClientError::ShuttingDown);
        }
        state.active = state.active.saturating_add(1);
        drop(state);

        Ok(OperationGuard {
            lifecycle: Arc::clone(self),
            cancellation_rx: self.cancellation_tx.subscribe(),
            _permit: permit,
        })
    }

    /// Seal admission and wait for the durable shutdown coordinator.
    pub(crate) async fn shutdown(self: &Arc<Self>, deadline: Instant) {
        self.seal(deadline);
        self.ensure_coordinator();
        self.wait_for_completion().await;
    }

    fn seal(&self, deadline: Instant) {
        let mut state = self.state.lock();
        if state.phase == Phase::Running {
            state.phase = Phase::Draining;
            state.drain_deadline = Some(deadline);
            self.semaphore.close();
            debug!(
                active = state.active,
                ?deadline,
                "sealed Modbus client admission"
            );
        }
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

        let lifecycle = Arc::clone(self);
        let handle = tokio::spawn(async move {
            lifecycle.coordinate_shutdown().await;
        });
        let abort = handle.abort_handle();
        drop(handle);
        self.tasks.lock().coordinator_abort = Some(abort);
    }

    async fn coordinate_shutdown(self: Arc<Self>) {
        let drain_deadline = {
            let state = self.state.lock();
            (state.phase == Phase::Draining)
                .then_some(state.drain_deadline)
                .flatten()
        };

        if let Some(deadline) = drain_deadline {
            tokio::select! {
                biased;
                () = self.wait_for_zero_active() => {}
                () = self.wait_for_cancellation() => {}
                () = tokio::time::sleep_until(deadline) => {
                    warn!(active = self.active_count(), "Modbus client shutdown deadline elapsed");
                    self.abort_inner();
                }
            }
        }

        let hard_stop = self.state.lock().phase == Phase::Cancelling;
        self.stop_tasks(hard_stop);
        self.join_tasks().await;

        self.connected.store(false, Ordering::Release);
        {
            let mut state = self.state.lock();
            state.phase = Phase::Closed;
        }
        self.completion_tx.send_replace(true);
        debug!("Modbus client shutdown complete");
    }

    async fn wait_for_zero_active(&self) {
        loop {
            let notified = self.active_changed.notified();
            if self.state.lock().active == 0 {
                return;
            }
            notified.await;
        }
    }

    async fn wait_for_cancellation(&self) {
        let mut receiver = self.cancellation_tx.subscribe();
        loop {
            if *receiver.borrow() {
                return;
            }
            if receiver.changed().await.is_err() {
                return;
            }
        }
    }

    async fn wait_for_completion(&self) {
        let mut receiver = self.completion_tx.subscribe();
        loop {
            if *receiver.borrow() {
                return;
            }
            if receiver.changed().await.is_err() {
                return;
            }
        }
    }

    /// Immediately seal admission, cancel work, and request task termination.
    pub(crate) fn abort(&self) {
        self.abort_inner();
    }

    fn abort_inner(&self) {
        {
            let mut state = self.state.lock();
            if state.phase != Phase::Closed {
                state.phase = Phase::Cancelling;
            }
            self.semaphore.close();
        }
        self.connected.store(false, Ordering::Release);
        self.txn_mgr.cancel_all(|| ClientError::ShuttingDown);
        self.cancellation_tx.send_replace(true);
        self.task_stop_tx.send_replace(true);
        self.abort_tasks();
    }

    fn stop_tasks(&self, hard_stop: bool) {
        self.task_stop_tx.send_replace(true);
        let tasks = self.tasks.lock();
        if hard_stop && let Some(abort) = &tasks.reader_abort {
            abort.abort();
        }
        if let Some(abort) = &tasks.deadline_abort {
            abort.abort();
        }
    }

    fn abort_tasks(&self) {
        let tasks = self.tasks.lock();
        if let Some(abort) = &tasks.reader_abort {
            abort.abort();
        }
        if let Some(abort) = &tasks.deadline_abort {
            abort.abort();
        }
    }

    async fn join_tasks(&self) {
        let (reader, deadline) = {
            let mut tasks = self.tasks.lock();
            (tasks.reader.take(), tasks.deadline.take())
        };

        if let Some(handle) = reader
            && let Err(error) = handle.await
            && !error.is_cancelled()
        {
            warn!(%error, "Modbus reader task failed during shutdown");
        }
        if let Some(handle) = deadline
            && let Err(error) = handle.await
            && !error.is_cancelled()
        {
            warn!(%error, "Modbus deadline task failed during shutdown");
        }
    }

    /// Abort a detached shutdown coordinator during final client drop.
    pub(crate) fn abort_coordinator(&self) {
        if let Some(abort) = &self.tasks.lock().coordinator_abort {
            abort.abort();
        }
    }
}

/// RAII ownership for one admitted logical operation.
pub(crate) struct OperationGuard {
    lifecycle: Arc<ClientLifecycle>,
    cancellation_rx: watch::Receiver<bool>,
    _permit: OwnedSemaphorePermit,
}

impl OperationGuard {
    /// Whether hard cancellation has already been requested.
    pub(crate) fn is_cancelled(&self) -> bool {
        *self.cancellation_rx.borrow()
    }

    /// Wait for sticky hard cancellation.
    pub(crate) async fn cancelled(&mut self) {
        loop {
            if *self.cancellation_rx.borrow() {
                return;
            }
            if self.cancellation_rx.changed().await.is_err() {
                return;
            }
        }
    }
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        let mut state = self.lifecycle.state.lock();
        debug_assert!(state.active > 0, "operation count underflow");
        state.active = state.active.saturating_sub(1);
        let became_idle = state.active == 0;
        drop(state);
        if became_idle {
            self.lifecycle.active_changed.notify_one();
        }
    }
}
