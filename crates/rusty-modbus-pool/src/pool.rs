//! Connection pool — two-pool model per TCP Guide §4.2.1.

use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::{Notify, watch};
use tokio::time::Instant;

use rusty_modbus_tcp::{
    TcpIdleObservation, TcpRecvStream, TcpSink, TcpTransport, TransportConnect, inspect_idle_tcp,
};

use crate::backoff::Backoff;
use crate::config::PoolConfig;
use crate::connection::PooledConnection;
use crate::error::PoolError;
use crate::health;

/// A pooled connection entry stored internally.
pub(crate) struct PoolEntry {
    /// Remote address this connection is connected to.
    pub addr: SocketAddr,
    /// Write half.
    pub sink: TcpSink,
    /// Read half.
    pub stream: TcpRecvStream,
    /// When this connection was last returned to the pool.
    pub last_used: Instant,
    /// Whether this connection is to a priority device.
    pub is_priority: bool,
}

/// Bounded source of one passive idle retirement event.
#[derive(Clone, Copy)]
pub(crate) enum IdleValidationTrigger {
    Checkout,
    HealthSweep,
}

impl IdleValidationTrigger {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Checkout => "checkout",
            Self::HealthSweep => "health_sweep",
        }
    }
}

/// Result of inspecting one idle entry without consuming its receive data.
pub(crate) enum IdleEntryInspection {
    Retain(PoolEntry),
    Retire(PoolEntry, TcpIdleObservation),
}

/// Inspect one idle entry without consuming its receive data.
pub(crate) fn inspect_idle_entry(entry: PoolEntry) -> IdleEntryInspection {
    let PoolEntry {
        addr,
        sink,
        stream,
        last_used,
        is_priority,
    } = entry;
    let (sink, stream, observation) = inspect_idle_tcp(sink, stream);
    let entry = PoolEntry {
        addr,
        sink,
        stream,
        last_used,
        is_priority,
    };

    if matches!(observation, TcpIdleObservation::NoAdverseSignal) {
        IdleEntryInspection::Retain(entry)
    } else {
        IdleEntryInspection::Retire(entry, observation)
    }
}

/// Emit exactly one bounded event per passive retirement, then drop all entries
/// outside the pool mutex and wake capacity waiters.
pub(crate) fn finish_passive_retirements(
    retirements: Vec<(PoolEntry, TcpIdleObservation)>,
    trigger: IdleValidationTrigger,
    capacity_changed: &Notify,
) {
    if retirements.is_empty() {
        return;
    }

    for (entry, observation) in retirements {
        tracing::debug!(
            target: "rusty_modbus_pool::idle_validation",
            reason = idle_observation_reason(observation),
            trigger = trigger.as_str(),
            is_priority = entry.is_priority,
            "idle_tcp_connection_passively_retired"
        );
        drop(entry);
    }
    capacity_changed.notify_waiters();
}

const fn idle_observation_reason(observation: TcpIdleObservation) -> &'static str {
    match observation {
        TcpIdleObservation::NoAdverseSignal => "none",
        TcpIdleObservation::QueuedInput => "queued_input",
        TcpIdleObservation::PeerClosed => "peer_closed",
        TcpIdleObservation::SocketError(_) => "socket_error",
        TcpIdleObservation::MismatchedHalves => "mismatched_halves",
        _ => "other",
    }
}

/// Shared mutable pool state.
///
/// The two pools are accounted **separately** (TCP Guide §4.2.1): non-priority
/// connections are bounded by [`PoolConfig::max_connections`], while each
/// priority device has its own budget ([`PriorityDevice::max_connections`](crate::PriorityDevice::max_connections)).
/// Keeping them separate is what stops idle priority connections that are not
/// age- or capacity-evicted from starving non-priority requests.
pub(crate) struct PoolInner {
    /// Idle connections available for reuse (entries from both pools).
    pub idle: Vec<PoolEntry>,
    /// Number of active accounting charges for **non-priority** connections,
    /// including checked-out connections and pending connection establishment.
    pub active_non_priority: usize,
    /// Number of active accounting charges for **priority** connections,
    /// including checked-out connections and pending connection establishment,
    /// counted per device address so each device's budget is independent.
    pub active_priority: HashMap<SocketAddr, usize>,
    /// Whether the pool is shutting down.
    pub shutting_down: bool,
}

impl PoolInner {
    /// Total active accounting charges across both pools, including pending
    /// connection establishment.
    pub(crate) fn active_total(&self) -> usize {
        self.active_non_priority + self.active_priority.values().sum::<usize>()
    }

    /// Active + idle connections to a specific priority device (its pool size).
    pub(crate) fn priority_total(&self, addr: SocketAddr) -> usize {
        let active = self.active_priority.get(&addr).copied().unwrap_or(0);
        let idle = self.priority_idle_count(addr);
        active + idle
    }

    /// Idle priority connections to one configured device address.
    pub(crate) fn priority_idle_count(&self, addr: SocketAddr) -> usize {
        self.idle
            .iter()
            .filter(|entry| entry.is_priority && entry.addr == addr)
            .count()
    }

    /// Active + idle non-priority connections (the non-priority pool size).
    pub(crate) fn non_priority_total(&self) -> usize {
        let idle = self.idle.iter().filter(|e| !e.is_priority).count();
        self.active_non_priority + idle
    }

    /// Increment the active counter for a connection being checked out.
    pub(crate) fn charge_active(&mut self, entry: &PoolEntry) {
        if entry.is_priority {
            *self.active_priority.entry(entry.addr).or_insert(0) += 1;
        } else {
            self.active_non_priority += 1;
        }
    }

    /// Decrement the active counter when a connection retires, returns to idle,
    /// or a pending connect attempt fails.
    pub(crate) fn release_active(&mut self, is_priority: bool, addr: SocketAddr) {
        if is_priority {
            if let Some(c) = self.active_priority.get_mut(&addr) {
                *c = c.saturating_sub(1);
                if *c == 0 {
                    self.active_priority.remove(&addr);
                }
            }
        } else {
            self.active_non_priority = self.active_non_priority.saturating_sub(1);
        }
    }
}

/// Owns an active capacity charge while a new connection is pending.
///
/// Unless committed into a [`PooledConnection`] or an idle priority-maintenance
/// entry, dropping the reservation releases the charge and restores any idle
/// entry tentatively evicted to make room. Rollback is synchronous so dropping
/// a pending connector future is enough to restore the pool invariants.
struct PendingReservation {
    pool: Arc<Mutex<PoolInner>>,
    capacity_changed: Arc<Notify>,
    addr: SocketAddr,
    is_priority: bool,
    evicted: Option<PoolEntry>,
    committed: bool,
}

impl PendingReservation {
    fn new(
        pool: Arc<Mutex<PoolInner>>,
        capacity_changed: Arc<Notify>,
        addr: SocketAddr,
        is_priority: bool,
        evicted: Option<PoolEntry>,
    ) -> Self {
        Self {
            pool,
            capacity_changed,
            addr,
            is_priority,
            evicted,
            committed: false,
        }
    }

    /// Transfer ownership of the active charge to the returned lease.
    fn commit(mut self, entry: PoolEntry) -> PooledConnection {
        let connection = PooledConnection::new(
            entry,
            Arc::clone(&self.pool),
            Arc::clone(&self.capacity_changed),
        );
        self.committed = true;
        // A successful replacement deliberately retires the evicted connection.
        drop(self.evicted.take());
        connection
    }

    /// Atomically convert a pending priority charge into an idle entry.
    ///
    /// Releasing the active charge and inserting idle happen under one lock so
    /// another acquisition can never observe a transient hole in the device
    /// budget. If shutdown won, the transport is dropped after unlocking.
    fn commit_priority_idle_if_needed(mut self, entry: PoolEntry) {
        debug_assert!(self.is_priority);
        debug_assert!(self.evicted.is_none());
        debug_assert!(entry.is_priority);
        debug_assert_eq!(entry.addr, self.addr);

        let rejected = {
            let mut inner = self.pool.lock();
            inner.release_active(self.is_priority, self.addr);
            self.committed = true;
            if inner.shutting_down || inner.priority_idle_count(self.addr) != 0 {
                Some(entry)
            } else {
                inner.idle.push(entry);
                None
            }
        };

        self.capacity_changed.notify_waiters();
        drop(rejected);
    }
}

impl Drop for PendingReservation {
    fn drop(&mut self) {
        if self.committed {
            return;
        }

        let mut inner = self.pool.lock();
        inner.release_active(self.is_priority, self.addr);
        if !inner.shutting_down
            && let Some(entry) = self.evicted.take()
        {
            // Preserve the original entry, including its LRU timestamp.
            inner.idle.push(entry);
        }
        drop(inner);
        self.capacity_changed.notify_waiters();
    }
}

/// Capacity acquired either by checking out an idle entry or by reserving a
/// charge for a new connection.
enum Acquisition {
    Reused(PooledConnection),
    Reserved(PendingReservation),
}

/// Join handles for every task whose lifecycle is owned by the pool.
struct OwnedBackgroundTasks {
    health: tokio::task::JoinHandle<()>,
    priority_maintenance: Vec<tokio::task::JoinHandle<()>>,
}

impl OwnedBackgroundTasks {
    /// Hard-abort only the health task without taking ownership of its handle.
    fn abort_health(&self) {
        self.health.abort();
    }

    /// Abort health defensively, join every task, then publish sticky completion.
    async fn join_and_complete(self, completed: watch::Sender<bool>) {
        let priority_maintenance_task_count = self.priority_maintenance.len();
        let task_count = priority_maintenance_task_count.saturating_add(1);
        tracing::debug!(
            target: "rusty_modbus_pool::shutdown",
            task_count,
            priority_maintenance_task_count,
            "pool_background_task_join_started"
        );

        self.abort_health();
        let Self {
            health,
            priority_maintenance,
        } = self;
        let mut unexpected_join_errors = 0_usize;

        if let Err(error) = health.await
            && !error.is_cancelled()
        {
            unexpected_join_errors += 1;
        }
        for task in priority_maintenance {
            if let Err(error) = task.await
                && !error.is_cancelled()
            {
                unexpected_join_errors += 1;
            }
        }

        if unexpected_join_errors != 0 {
            tracing::warn!(
                target: "rusty_modbus_pool::shutdown",
                unexpected_join_errors,
                task_count,
                "pool_background_task_join_failed"
            );
        }
        tracing::debug!(
            target: "rusty_modbus_pool::shutdown",
            task_count,
            priority_maintenance_task_count,
            "pool_background_task_join_completed"
        );
        completed.send_replace(true);
    }
}

/// Shared ownership handoff and sticky completion state for pool task shutdown.
struct BackgroundShutdownState {
    tasks: Option<OwnedBackgroundTasks>,
    coordinator_started: bool,
    completed: watch::Sender<bool>,
}

/// Connection pool with two-pool eviction model.
///
/// Priority connections (to configured addresses) are never age- or
/// capacity-evicted and live in a per-device budget separate from the
/// non-priority pool. Known-adverse idle priority transports are still retired.
/// Non-priority connections are evicted oldest-first when their pool is full.
/// Capacity waiters use change broadcasts only as retry hints; the pool makes no
/// fairness or FIFO guarantee, and `PoolInner` remains the accounting authority.
pub struct ConnectionPool {
    inner: Arc<Mutex<PoolInner>>,
    capacity_changed: Arc<Notify>,
    config: PoolConfig,
    priority_maintenance_stop: watch::Sender<bool>,
    background_shutdown: Mutex<BackgroundShutdownState>,
}

impl std::fmt::Debug for ConnectionPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionPool")
            .field("active", &self.active_count())
            .field("idle", &self.idle_count())
            .field("max", &self.config.max_connections)
            .finish_non_exhaustive()
    }
}

impl ConnectionPool {
    /// Create a new connection pool.
    ///
    /// Priority background maintenance is disabled only when both
    /// `config.pre_connect` and `config.priority_replenishment` are false. Initial
    /// warm-up alone exits after its one-idle target is met or unavailable under
    /// the per-device cap. Replenishment keeps one task per distinct address to
    /// restore that target after checkout or retirement. The call itself returns
    /// immediately.
    #[must_use]
    pub fn new(config: PoolConfig) -> Self {
        Self::new_with_priority_maintenance_runtime(
            config,
            TcpTransport::connect,
            tokio::time::sleep,
            tokio::time::sleep,
        )
    }

    /// Construction path with injectable priority connector, retry backoff, and
    /// standing-policy fallback wait.
    fn new_with_priority_maintenance_runtime<C, F, R, RF, W, WF>(
        config: PoolConfig,
        connector: C,
        retry_wait: R,
        fallback_wait: W,
    ) -> Self
    where
        C: Fn(rusty_modbus_tcp::TcpConfig, SocketAddr) -> F + Clone + Send + 'static,
        F: Future<Output = Result<(TcpSink, TcpRecvStream), rusty_modbus_tcp::TransportError>>
            + Send
            + 'static,
        R: Fn(Duration) -> RF + Clone + Send + 'static,
        RF: Future<Output = ()> + Send + 'static,
        W: Fn(Duration) -> WF + Clone + Send + 'static,
        WF: Future<Output = ()> + Send + 'static,
    {
        let inner = Arc::new(Mutex::new(PoolInner {
            idle: Vec::new(),
            active_non_priority: 0,
            active_priority: HashMap::new(),
            shutting_down: false,
        }));
        let capacity_changed = Arc::new(Notify::new());

        let health_task = health::spawn_health_check(
            Arc::clone(&inner),
            Arc::clone(&capacity_changed),
            config.health_check_interval,
            config.idle_timeout,
        );
        let (priority_maintenance_stop, priority_maintenance_stop_rx) = watch::channel(false);

        let priority_maintenance_tasks = if config.pre_connect || config.priority_replenishment {
            Self::spawn_priority_maintenance(
                &config,
                &inner,
                &capacity_changed,
                connector,
                retry_wait,
                fallback_wait,
                &priority_maintenance_stop_rx,
            )
        } else {
            Vec::new()
        };
        let (completed, _) = watch::channel(false);

        Self {
            inner,
            capacity_changed,
            config,
            priority_maintenance_stop,
            background_shutdown: Mutex::new(BackgroundShutdownState {
                tasks: Some(OwnedBackgroundTasks {
                    health: health_task,
                    priority_maintenance: priority_maintenance_tasks,
                }),
                coordinator_started: false,
                completed,
            }),
        }
    }

    /// Get a connection to the specified address.
    ///
    /// Returns an idle connection if available, otherwise establishes a new one.
    /// This method is fail-fast: it does not wait for pool capacity.
    /// Priority devices draw from their own per-device budget; non-priority
    /// requests draw from `max_connections`, evicting the oldest idle
    /// non-priority connection when that pool is full.
    ///
    /// # Errors
    ///
    /// - [`PoolError::ShuttingDown`] if the pool is shutting down.
    /// - [`PoolError::Exhausted`] if the relevant pool is full and no connection
    ///   can be reused or capacity-evicted (priority devices are not
    ///   capacity-evicted, so a priority request fails once the device hits its
    ///   own `max_connections`).
    /// - [`PoolError::ConnectionFailed`] if establishing a new connection fails.
    pub async fn get(&self, addr: SocketAddr) -> Result<PooledConnection, PoolError> {
        let tcp_config = self.config.tcp_config.clone();
        self.get_with_connector(addr, move || TcpTransport::connect(tcp_config, addr))
            .await
    }

    /// Get a connection, waiting up to `timeout` for pool capacity when full.
    ///
    /// Idle reuse or a new capacity reservation is attempted immediately, even
    /// when `timeout` is [`Duration::ZERO`]. Only a full relevant priority or
    /// non-priority budget causes a wait. The fixed acquisition deadline ends as
    /// soon as capacity is reserved and never wraps TCP connection establishment;
    /// transport connection timeouts remain reported through
    /// [`PoolError::ConnectionFailed`].
    ///
    /// Waiting is cancellation-safe and shutdown-aware. Capacity-change
    /// broadcasts are stateless retry hints, so wakeups may be spurious or cross
    /// pool budgets and no fairness or FIFO ordering is guaranteed.
    ///
    /// If `timeout` is too large to represent as an absolute deadline, the
    /// initial acquisition attempt is still honored. If the relevant budget
    /// remains full after a final state check, this method returns
    /// [`PoolError::Timeout`] rather than panicking or waiting without a bound.
    ///
    /// # Errors
    ///
    /// - [`PoolError::ShuttingDown`] if shutdown is observed, including while
    ///   waiting.
    /// - [`PoolError::Timeout`] if capacity is not reserved by the fixed deadline.
    /// - [`PoolError::ConnectionFailed`] if a post-reservation TCP connection
    ///   attempt fails.
    pub async fn get_with_acquisition_timeout(
        &self,
        addr: SocketAddr,
        timeout: Duration,
    ) -> Result<PooledConnection, PoolError> {
        let tcp_config = self.config.tcp_config.clone();
        self.get_with_acquisition_timeout_and_connector(addr, timeout, move || {
            TcpTransport::connect(tcp_config, addr)
        })
        .await
    }

    /// Internal acquisition path with an injectable connector for deterministic
    /// cancellation testing.
    async fn get_with_connector<C, F>(
        &self,
        addr: SocketAddr,
        connector: C,
    ) -> Result<PooledConnection, PoolError>
    where
        C: FnOnce() -> F,
        F: Future<Output = Result<(TcpSink, TcpRecvStream), rusty_modbus_tcp::TransportError>>,
    {
        let acquisition = self.acquire_immediate(addr)?;
        Self::finish_acquisition(acquisition, connector).await
    }

    /// Timed acquisition path with an injectable connector for deterministic
    /// capacity-wait and connector-scope testing.
    async fn get_with_acquisition_timeout_and_connector<C, F>(
        &self,
        addr: SocketAddr,
        timeout: Duration,
        connector: C,
    ) -> Result<PooledConnection, PoolError>
    where
        C: FnOnce() -> F,
        F: Future<Output = Result<(TcpSink, TcpRecvStream), rusty_modbus_tcp::TransportError>>,
    {
        let acquisition = self.acquire_with_timeout(addr, timeout).await?;
        Self::finish_acquisition(acquisition, connector).await
    }

    /// Attempt one immediate idle checkout or reservation.
    fn acquire_immediate(&self, addr: SocketAddr) -> Result<Acquisition, PoolError> {
        let is_priority = self.is_priority_addr(addr);
        let mut retirements = Vec::new();
        let acquisition = {
            let mut inner = self.inner.lock();
            if inner.shutting_down {
                return Err(PoolError::ShuttingDown);
            }

            self.try_acquire_locked(&mut inner, addr, is_priority, &mut retirements)
        };
        finish_passive_retirements(
            retirements,
            IdleValidationTrigger::Checkout,
            &self.capacity_changed,
        );
        acquisition.ok_or(PoolError::Exhausted)
    }

    /// Wait for an idle checkout or reservation using one absolute deadline.
    async fn acquire_with_timeout(
        &self,
        addr: SocketAddr,
        timeout: Duration,
    ) -> Result<Acquisition, PoolError> {
        let is_priority = self.is_priority_addr(addr);

        // Always honor one shutdown-aware immediate attempt before constructing
        // the deadline. Besides preserving zero-timeout behavior, this keeps an
        // unrepresentably large timeout from rejecting available capacity.
        let mut retirements = Vec::new();
        let initial_acquisition = {
            let mut inner = self.inner.lock();
            if inner.shutting_down {
                return Err(PoolError::ShuttingDown);
            }
            self.try_acquire_locked(&mut inner, addr, is_priority, &mut retirements)
        };
        finish_passive_retirements(
            retirements,
            IdleValidationTrigger::Checkout,
            &self.capacity_changed,
        );
        if let Some(acquisition) = initial_acquisition {
            return Ok(acquisition);
        }

        let Some(deadline) = Instant::now().checked_add(timeout) else {
            // Capacity may have changed after the initial full observation. Check
            // complete state once more, with sticky shutdown taking precedence,
            // before returning the deterministic overflow result.
            let mut retirements = Vec::new();
            let acquisition = {
                let mut inner = self.inner.lock();
                if inner.shutting_down {
                    return Err(PoolError::ShuttingDown);
                }
                self.try_acquire_locked(&mut inner, addr, is_priority, &mut retirements)
            };
            finish_passive_retirements(
                retirements,
                IdleValidationTrigger::Checkout,
                &self.capacity_changed,
            );
            return acquisition.ok_or(PoolError::Timeout);
        };
        let mut waited = false;

        loop {
            // Enable registration before inspecting state so a broadcast between
            // the inspection and await cannot be lost. Notify is only a hint;
            // every wake retries the exact budget under PoolInner.
            let notified = self.capacity_changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            let mut retirements = Vec::new();
            let acquisition = {
                let mut inner = self.inner.lock();
                if inner.shutting_down {
                    return Err(PoolError::ShuttingDown);
                }
                // Once any wait has occurred, shutdown wins and then the fixed
                // deadline wins before newly available capacity can be admitted.
                if waited && Instant::now() >= deadline {
                    return Err(PoolError::Timeout);
                }
                self.try_acquire_locked(&mut inner, addr, is_priority, &mut retirements)
            };
            finish_passive_retirements(
                retirements,
                IdleValidationTrigger::Checkout,
                &self.capacity_changed,
            );

            if let Some(acquisition) = acquisition {
                return Ok(acquisition);
            }

            waited = true;
            tokio::select! {
                () = notified.as_mut() => {}
                () = tokio::time::sleep_until(deadline) => {
                    // Recheck sticky shutdown at the deadline so it wins whenever
                    // it has been observed by the pool state machine.
                    if self.inner.lock().shutting_down {
                        return Err(PoolError::ShuttingDown);
                    }
                    return Err(PoolError::Timeout);
                }
            }
        }
    }

    /// Inspect and mutate capacity while the caller holds the pool mutex.
    fn try_acquire_locked(
        &self,
        inner: &mut PoolInner,
        addr: SocketAddr,
        is_priority: bool,
        retirements: &mut Vec<(PoolEntry, TcpIdleObservation)>,
    ) -> Option<Acquisition> {
        // Validate every matching idle candidate before charging it active.
        // Inspection performs one non-waiting, non-consuming socket peek while
        // the mutex keeps the exact entry from being concurrently checked out.
        while let Some(idx) = inner.idle.iter().position(|entry| entry.addr == addr) {
            let entry = inner.idle.swap_remove(idx);
            match inspect_idle_entry(entry) {
                IdleEntryInspection::Retain(mut entry) => {
                    entry.last_used = Instant::now();
                    inner.charge_active(&entry);
                    if entry.is_priority {
                        // This is the single idle-checkout source for fail-fast
                        // and timed acquisition. Wake a standing maintainer after
                        // its one-idle target became active.
                        self.capacity_changed.notify_waiters();
                    }
                    return Some(Acquisition::Reused(PooledConnection::new(
                        entry,
                        Arc::clone(&self.inner),
                        Arc::clone(&self.capacity_changed),
                    )));
                }
                IdleEntryInspection::Retire(entry, observation) => {
                    retirements.push((entry, observation));
                }
            }
        }

        // No idle connection: reserve a slot in the appropriate pool.
        let evicted = if is_priority {
            // Priority pool: bounded by this device's own budget and never
            // capacity-evicted. Passive adverse-signal retirement happened above.
            let cap = self.priority_cap(addr);
            if inner.priority_total(addr) >= cap {
                return None;
            }
            *inner.active_priority.entry(addr).or_insert(0) += 1;
            None
        } else {
            // Non-priority pool: bounded by max_connections, LRU-evicted.
            if inner.non_priority_total() < self.config.max_connections {
                inner.active_non_priority += 1;
                None
            } else if let Some(idx) = find_evictable(inner) {
                // Evict to stay within the cap, but keep the entry in hand:
                // rollback restores it unchanged.
                let evicted = inner.idle.swap_remove(idx);
                inner.active_non_priority += 1;
                Some(evicted)
            } else {
                return None;
            }
        };

        // Establish the RAII owner before releasing the pool lock.
        Some(Acquisition::Reserved(PendingReservation::new(
            Arc::clone(&self.inner),
            Arc::clone(&self.capacity_changed),
            addr,
            is_priority,
            evicted,
        )))
    }

    /// Reuse an idle lease or run the connector after a reservation. No pool
    /// acquisition deadline reaches this phase.
    async fn finish_acquisition<C, F>(
        acquisition: Acquisition,
        connector: C,
    ) -> Result<PooledConnection, PoolError>
    where
        C: FnOnce() -> F,
        F: Future<Output = Result<(TcpSink, TcpRecvStream), rusty_modbus_tcp::TransportError>>,
    {
        match acquisition {
            Acquisition::Reused(connection) => Ok(connection),
            Acquisition::Reserved(reservation) => {
                // Connect outside the lock while `reservation` owns the active
                // charge and any tentatively evicted entry.
                match connector().await {
                    Ok((sink, stream)) => {
                        let entry = PoolEntry {
                            addr: reservation.addr,
                            sink,
                            stream,
                            last_used: Instant::now(),
                            is_priority: reservation.is_priority,
                        };
                        Ok(reservation.commit(entry))
                    }
                    Err(e) => {
                        // Use the same guard rollback as future cancellation.
                        drop(reservation);
                        Err(PoolError::ConnectionFailed(e))
                    }
                }
            }
        }
    }

    /// Shut down the pool synchronously without waiting for background tasks.
    ///
    /// This immediately and idempotently seals admission, drops idle connections,
    /// wakes capacity waiters, publishes the sticky cooperative stop for priority
    /// maintenance, and hard-aborts the pool-owned health check. It does not wait
    /// for those tasks or their cancellation destructors. Use
    /// [`shutdown_and_wait`](Self::shutdown_and_wait) when proof of that bounded
    /// task quiescence is required.
    ///
    /// Checked-out leases, reusable client sessions, and caller-owned pending
    /// demand connector futures are outside both shutdown methods' wait boundary.
    pub fn shutdown(&self) {
        {
            let mut inner = self.inner.lock();
            inner.shutting_down = true;
            inner.idle.clear();
        }
        self.capacity_changed.notify_waiters();
        self.priority_maintenance_stop.send_replace(true);

        if let Some(tasks) = &self.background_shutdown.lock().tasks {
            tasks.abort_health();
        }
    }

    /// Shut down the pool and wait for all pool-owned background tasks to terminate.
    ///
    /// The first polled caller starts one detached coordinator that owns and joins
    /// the health-check and priority-maintenance task handles. Health is hard-
    /// aborted, while priority maintenance is joined only after observing its
    /// cooperative sticky stop. Completion is sticky and shared by concurrent and
    /// later callers. Cancelling a caller does not cancel the coordinator or
    /// prevent a later caller from observing completion. When this method returns,
    /// cleanup for those tasks, including any priority-maintenance reservation
    /// rollback, has completed.
    ///
    /// This bounded quiescence excludes checked-out raw leases, reusable client
    /// sessions, and caller-owned pending demand connector futures. Consequently,
    /// [`active_count`](Self::active_count) may remain nonzero after this method
    /// returns. It does not prove that every reference to pool accounting is gone.
    ///
    /// The future must be polled inside a live Tokio runtime. Runtime teardown can
    /// cancel runtime tasks and therefore cannot promise asynchronous completion;
    /// [`shutdown`](Self::shutdown) and [`Drop`] remain nonblocking in that case.
    ///
    /// # Panics
    ///
    /// Panics if the first caller that starts the detached coordinator is polled
    /// outside a Tokio runtime.
    pub async fn shutdown_and_wait(&self) {
        self.shutdown();

        let (mut completion, tasks, completed) = {
            let mut shutdown = self.background_shutdown.lock();
            let completion = shutdown.completed.subscribe();
            let tasks = if shutdown.coordinator_started {
                None
            } else {
                shutdown.coordinator_started = true;
                Some(
                    shutdown
                        .tasks
                        .take()
                        .expect("background tasks must exist before coordinator start"),
                )
            };
            (completion, tasks, shutdown.completed.clone())
        };

        if let Some(tasks) = tasks {
            tokio::spawn(tasks.join_and_complete(completed));
        }

        loop {
            if *completion.borrow() {
                return;
            }
            completion
                .changed()
                .await
                .expect("pool shutdown completion sender must outlive waiters");
        }
    }

    /// Number of active accounting charges across both pools.
    ///
    /// This includes checked-out connections and capacity reserved while a
    /// demand or priority-maintenance connector is pending. It excludes
    /// idle connections.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.inner.lock().active_total()
    }

    /// Number of idle connections in the pool.
    #[must_use]
    pub fn idle_count(&self) -> usize {
        self.inner.lock().idle.len()
    }

    /// Check if an address is a priority device.
    fn is_priority_addr(&self, addr: SocketAddr) -> bool {
        self.config
            .priority_devices
            .iter()
            .any(|pd| pd.addr == addr)
    }

    /// The per-device connection cap for a priority address (0 if not configured).
    fn priority_cap(&self, addr: SocketAddr) -> usize {
        self.config
            .priority_devices
            .iter()
            .find(|pd| pd.addr == addr)
            .map_or(0, |pd| pd.max_connections)
    }

    /// Spawn background tasks to warm or maintain priority devices.
    ///
    /// One task per *distinct* device address (duplicate `priority_devices`
    /// entries for the same address are ignored so they cannot collectively
    /// exceed the first entry's per-device budget). One-shot tasks exit when the
    /// one-idle target is unavailable or met. Standing tasks wait on capacity
    /// changes plus a safety-only fallback, then replenish the target without
    /// probing an existing socket. Connector failures always observe exponential
    /// [`Backoff`]. Each returned task cooperatively observes the pool's sticky
    /// maintenance-stop signal; [`shutdown`](Self::shutdown) does not abort them.
    fn spawn_priority_maintenance<C, F, R, RF, W, WF>(
        config: &PoolConfig,
        inner: &Arc<Mutex<PoolInner>>,
        capacity_changed: &Arc<Notify>,
        connector: C,
        retry_wait: R,
        fallback_wait: W,
        maintenance_stop: &watch::Receiver<bool>,
    ) -> Vec<tokio::task::JoinHandle<()>>
    where
        C: Fn(rusty_modbus_tcp::TcpConfig, SocketAddr) -> F + Clone + Send + 'static,
        F: Future<Output = Result<(TcpSink, TcpRecvStream), rusty_modbus_tcp::TransportError>>
            + Send
            + 'static,
        R: Fn(Duration) -> RF + Clone + Send + 'static,
        RF: Future<Output = ()> + Send + 'static,
        W: Fn(Duration) -> WF + Clone + Send + 'static,
        WF: Future<Output = ()> + Send + 'static,
    {
        let mut seen = std::collections::HashSet::new();
        let mut handles = Vec::new();
        let mode = if config.priority_replenishment {
            PriorityMaintenanceMode::Standing
        } else {
            PriorityMaintenanceMode::OneShot
        };
        let fallback_interval = config
            .health_check_interval
            .max(MIN_PRIORITY_MAINTENANCE_SLEEP);

        for pd in &config.priority_devices {
            if !seen.insert(pd.addr) {
                continue; // already spawning for this address
            }
            let addr = pd.addr;
            let cap = pd.max_connections;
            if cap == 0 {
                continue;
            }
            let tcp_config = config.tcp_config.clone();
            let backoff_config = config.backoff.clone();
            let inner = Arc::clone(inner);
            let capacity_changed = Arc::clone(capacity_changed);
            let connector = connector.clone();
            let retry_wait = retry_wait.clone();
            let fallback_wait = fallback_wait.clone();
            let mut maintenance_stop = maintenance_stop.clone();

            handles.push(tokio::spawn(async move {
                let mut backoff = Backoff::new(backoff_config);
                loop {
                    if priority_maintenance_should_stop(&maintenance_stop) {
                        return;
                    }

                    // Register before inspecting policy/capacity so a checkout or
                    // retirement broadcast cannot be lost before the wait below.
                    let notified = capacity_changed.notified();
                    tokio::pin!(notified);
                    notified.as_mut().enable();

                    // PoolInner is the only policy and accounting authority. The
                    // pending reservation owns the exact active charge while the
                    // connector runs outside this lock.
                    let reservation = match reserve_priority_maintenance(
                        &inner,
                        &capacity_changed,
                        addr,
                        cap,
                    ) {
                        PriorityMaintenanceReservation::ShuttingDown => return,
                        PriorityMaintenanceReservation::Reserved(reservation) => reservation,
                        PriorityMaintenanceReservation::Unavailable => {
                            if matches!(mode, PriorityMaintenanceMode::OneShot) {
                                return;
                            }
                            tokio::select! {
                                biased;
                                () = wait_for_priority_maintenance_stop(&mut maintenance_stop) => return,
                                () = notified.as_mut() => {}
                                () = fallback_wait(fallback_interval) => {}
                            }
                            continue;
                        }
                    };

                    let connect_result = tokio::select! {
                        biased;
                        () = wait_for_priority_maintenance_stop(&mut maintenance_stop) => return,
                        result = connector(tcp_config.clone(), addr) => result,
                    };

                    let connected = if let Ok((sink, stream)) = connect_result {
                        if priority_maintenance_should_stop(&maintenance_stop) {
                            drop((sink, stream));
                            drop(reservation);
                            return;
                        }

                        // Establishment resets retry history even if shutdown
                        // or a concurrent idle return makes this transport
                        // redundant at commit time.
                        backoff.reset();
                        reservation.commit_priority_idle_if_needed(PoolEntry {
                            addr,
                            sink,
                            stream,
                            last_used: Instant::now(),
                            is_priority: true,
                        });
                        if matches!(mode, PriorityMaintenanceMode::OneShot) {
                            return;
                        }
                        true
                    } else {
                        drop(reservation);
                        false
                    };

                    if connected {
                        continue;
                    }

                    // A failure always waits its own exponential backoff; it
                    // never races the rollback notification emitted by its
                    // reservation. Success continues immediately to a fresh
                    // policy evaluation with reset backoff state.
                    let delay = backoff.next_delay().max(MIN_PRIORITY_MAINTENANCE_SLEEP);
                    tokio::select! {
                        biased;
                        () = wait_for_priority_maintenance_stop(&mut maintenance_stop) => return,
                        () = retry_wait(delay) => {}
                    }
                }
            }));
        }

        handles
    }
}

#[derive(Clone, Copy)]
enum PriorityMaintenanceMode {
    OneShot,
    Standing,
}

enum PriorityMaintenanceReservation {
    ShuttingDown,
    Unavailable,
    Reserved(PendingReservation),
}

/// Reserve one priority-maintenance charge under the pool accounting authority.
fn reserve_priority_maintenance(
    inner: &Arc<Mutex<PoolInner>>,
    capacity_changed: &Arc<Notify>,
    addr: SocketAddr,
    cap: usize,
) -> PriorityMaintenanceReservation {
    let mut pool = inner.lock();
    if pool.shutting_down {
        return PriorityMaintenanceReservation::ShuttingDown;
    }
    if pool.priority_idle_count(addr) != 0 || pool.priority_total(addr) >= cap {
        return PriorityMaintenanceReservation::Unavailable;
    }

    *pool.active_priority.entry(addr).or_insert(0) += 1;
    PriorityMaintenanceReservation::Reserved(PendingReservation::new(
        Arc::clone(inner),
        Arc::clone(capacity_changed),
        addr,
        true,
        None,
    ))
}

/// Sticky cooperative-stop observation. Sender closure is also a stop request.
fn priority_maintenance_should_stop(stop: &watch::Receiver<bool>) -> bool {
    if *stop.borrow() {
        true
    } else {
        stop.has_changed().is_err()
    }
}

/// Wait until priority maintenance is stopped explicitly or its sender closes.
async fn wait_for_priority_maintenance_stop(stop: &mut watch::Receiver<bool>) {
    loop {
        if priority_maintenance_should_stop(stop) {
            return;
        }
        if stop.changed().await.is_err() {
            return;
        }
    }
}

/// Local lower bound for priority retry and replenishment-fallback sleeps. It
/// prevents zero-duration configuration from spinning without changing the
/// health task's own interval behavior.
const MIN_PRIORITY_MAINTENANCE_SLEEP: Duration = Duration::from_millis(1);

impl Drop for ConnectionPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Find the index of the oldest idle non-priority connection to evict.
fn find_evictable(inner: &PoolInner) -> Option<usize> {
    let mut oldest_idx = None;
    let mut oldest_time = Instant::now();

    for (i, entry) in inner.idle.iter().enumerate() {
        if !entry.is_priority && entry.last_used < oldest_time {
            oldest_time = entry.last_used;
            oldest_idx = Some(i);
        }
    }
    oldest_idx
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::future::pending;
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc as std_mpsc;
    use std::time::Duration;

    use rusty_modbus_frame::FrameHeader;
    use rusty_modbus_tcp::TransportStream;
    use rusty_modbus_tcp::{TcpConfig, TransportError};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::oneshot::error::TryRecvError;
    use tokio::sync::{Barrier, mpsc, oneshot};
    use tokio::task::JoinHandle;
    use tracing::field::{Field, Visit};
    use tracing::{Event, Level, Subscriber, dispatcher};
    use tracing_subscriber::layer::Context;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{Layer, registry};

    type GetTask = JoinHandle<Result<PooledConnection, PoolError>>;

    static TRACING_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    struct GatedConnectAttempt {
        addr: SocketAddr,
        complete: oneshot::Sender<GatedConnectCompletion>,
    }

    enum GatedConnectCompletion {
        SyntheticSuccess,
        Supplied((TcpSink, TcpRecvStream)),
        Failure,
    }

    struct GatedRetryWait {
        delay: Duration,
        complete: oneshot::Sender<()>,
    }

    struct GatedFallbackWait {
        delay: Duration,
        complete: oneshot::Sender<()>,
    }

    impl GatedConnectAttempt {
        fn succeed(self) {
            assert!(
                self.complete
                    .send(GatedConnectCompletion::SyntheticSuccess)
                    .is_ok(),
                "priority maintenance attempt should remain pending"
            );
        }

        fn succeed_with(self, halves: (TcpSink, TcpRecvStream)) {
            assert!(
                self.complete
                    .send(GatedConnectCompletion::Supplied(halves))
                    .is_ok(),
                "priority maintenance attempt should remain pending"
            );
        }

        fn fail(self) {
            assert!(
                self.complete.send(GatedConnectCompletion::Failure).is_ok(),
                "priority maintenance attempt should remain pending"
            );
        }
    }

    impl GatedRetryWait {
        fn resume(self) {
            self.complete
                .send(())
                .expect("priority maintenance retry wait should remain pending");
        }
    }

    impl GatedFallbackWait {
        fn resume(self) {
            self.complete
                .send(())
                .expect("priority maintenance fallback wait should remain pending");
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct CapturedIdleRetirement {
        message: String,
        reason: String,
        trigger: String,
        is_priority: bool,
    }

    #[derive(Default)]
    struct IdleRetirementVisitor {
        message: Option<String>,
        reason: Option<String>,
        trigger: Option<String>,
        is_priority: Option<bool>,
    }

    impl Visit for IdleRetirementVisitor {
        fn record_str(&mut self, field: &Field, value: &str) {
            match field.name() {
                "reason" => self.reason = Some(value.to_owned()),
                "trigger" => self.trigger = Some(value.to_owned()),
                _ => {}
            }
        }

        fn record_bool(&mut self, field: &Field, value: bool) {
            if field.name() == "is_priority" {
                self.is_priority = Some(value);
            }
        }

        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                self.message = Some(format!("{value:?}"));
            }
        }
    }

    #[derive(Clone, Default)]
    struct IdleRetirementCapture {
        events: Arc<StdMutex<Vec<CapturedIdleRetirement>>>,
    }

    impl<S> Layer<S> for IdleRetirementCapture
    where
        S: Subscriber,
    {
        fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
            if event.metadata().target() != "rusty_modbus_pool::idle_validation" {
                return;
            }

            let mut visitor = IdleRetirementVisitor::default();
            event.record(&mut visitor);
            self.events.lock().unwrap().push(CapturedIdleRetirement {
                message: visitor.message.expect("idle retirement message field"),
                reason: visitor.reason.expect("idle retirement reason field"),
                trigger: visitor.trigger.expect("idle retirement trigger field"),
                is_priority: visitor.is_priority.expect("idle retirement priority field"),
            });
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct CapturedShutdownWarning {
        message: String,
        unexpected_join_errors: u64,
        task_count: u64,
    }

    #[derive(Default)]
    struct ShutdownWarningVisitor {
        message: Option<String>,
        unexpected_join_errors: Option<u64>,
        task_count: Option<u64>,
    }

    impl Visit for ShutdownWarningVisitor {
        fn record_u64(&mut self, field: &Field, value: u64) {
            match field.name() {
                "unexpected_join_errors" => self.unexpected_join_errors = Some(value),
                "task_count" => self.task_count = Some(value),
                _ => {}
            }
        }

        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                self.message = Some(format!("{value:?}"));
            }
        }
    }

    #[derive(Clone, Default)]
    struct ShutdownWarningCapture {
        events: Arc<StdMutex<Vec<CapturedShutdownWarning>>>,
    }

    impl<S> Layer<S> for ShutdownWarningCapture
    where
        S: Subscriber,
    {
        fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
            if event.metadata().target() != "rusty_modbus_pool::shutdown"
                || *event.metadata().level() != Level::WARN
            {
                return;
            }

            let mut visitor = ShutdownWarningVisitor::default();
            event.record(&mut visitor);
            self.events.lock().unwrap().push(CapturedShutdownWarning {
                message: visitor.message.expect("shutdown warning message field"),
                unexpected_join_errors: visitor
                    .unexpected_join_errors
                    .expect("shutdown warning error count field"),
                task_count: visitor
                    .task_count
                    .expect("shutdown warning task count field"),
            });
        }
    }

    struct DropSignal(Option<oneshot::Sender<()>>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            if let Some(signal) = self.0.take() {
                let _ = signal.send(());
            }
        }
    }

    fn test_addr(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    fn test_config(max_connections: usize) -> PoolConfig {
        PoolConfig {
            max_connections,
            pre_connect: false,
            idle_timeout: Duration::from_hours(1),
            health_check_interval: Duration::from_hours(1),
            ..PoolConfig::default()
        }
    }

    fn pre_connect_config(priority_devices: Vec<crate::PriorityDevice>) -> PoolConfig {
        PoolConfig {
            pre_connect: true,
            priority_devices,
            backoff: crate::BackoffConfig {
                initial_delay: Duration::from_millis(10),
                max_delay: Duration::from_millis(10),
                multiplier: 1.0,
            },
            ..test_config(4)
        }
    }

    fn replenishment_config(priority_devices: Vec<crate::PriorityDevice>) -> PoolConfig {
        PoolConfig {
            pre_connect: false,
            priority_replenishment: true,
            priority_devices,
            backoff: crate::BackoffConfig {
                initial_delay: Duration::from_millis(10),
                max_delay: Duration::from_millis(20),
                multiplier: 2.0,
            },
            ..test_config(4)
        }
    }

    fn pool_with_gated_priority_maintenance(
        config: PoolConfig,
    ) -> (
        ConnectionPool,
        mpsc::UnboundedReceiver<GatedConnectAttempt>,
        mpsc::UnboundedReceiver<GatedRetryWait>,
        mpsc::UnboundedReceiver<GatedFallbackWait>,
    ) {
        let (attempt_tx, attempt_rx) = mpsc::unbounded_channel();
        let (retry_tx, retry_rx) = mpsc::unbounded_channel();
        let (fallback_tx, fallback_rx) = mpsc::unbounded_channel();
        let pool = ConnectionPool::new_with_priority_maintenance_runtime(
            config,
            move |_tcp_config, addr| {
                let attempt_tx = attempt_tx.clone();
                async move {
                    let (complete_tx, complete_rx) = oneshot::channel();
                    if attempt_tx
                        .send(GatedConnectAttempt {
                            addr,
                            complete: complete_tx,
                        })
                        .is_err()
                    {
                        return Err(TransportError::Disconnected);
                    }

                    match complete_rx.await {
                        Ok(GatedConnectCompletion::SyntheticSuccess) => {
                            Ok(connected_halves().await)
                        }
                        Ok(GatedConnectCompletion::Supplied(halves)) => Ok(halves),
                        Ok(GatedConnectCompletion::Failure) | Err(_) => {
                            Err(TransportError::Disconnected)
                        }
                    }
                }
            },
            move |delay| {
                let retry_tx = retry_tx.clone();
                async move {
                    let (complete_tx, complete_rx) = oneshot::channel();
                    if retry_tx
                        .send(GatedRetryWait {
                            delay,
                            complete: complete_tx,
                        })
                        .is_ok()
                    {
                        let _ = complete_rx.await;
                    }
                }
            },
            move |delay| {
                let fallback_tx = fallback_tx.clone();
                async move {
                    let (complete_tx, complete_rx) = oneshot::channel();
                    if fallback_tx
                        .send(GatedFallbackWait {
                            delay,
                            complete: complete_tx,
                        })
                        .is_ok()
                    {
                        let _ = complete_rx.await;
                    }
                }
            },
        );
        (pool, attempt_rx, retry_rx, fallback_rx)
    }

    async fn next_priority_maintenance_attempt(
        attempts: &mut mpsc::UnboundedReceiver<GatedConnectAttempt>,
    ) -> GatedConnectAttempt {
        tokio::time::timeout(Duration::from_secs(1), attempts.recv())
            .await
            .expect("priority maintenance connector should start promptly")
            .expect("priority maintenance connector channel should remain open")
    }

    async fn next_retry_wait(
        retries: &mut mpsc::UnboundedReceiver<GatedRetryWait>,
    ) -> GatedRetryWait {
        tokio::time::timeout(Duration::from_secs(1), retries.recv())
            .await
            .expect("priority maintenance retry wait should start promptly")
            .expect("priority maintenance retry channel should remain open")
    }

    async fn next_fallback_wait(
        fallbacks: &mut mpsc::UnboundedReceiver<GatedFallbackWait>,
    ) -> GatedFallbackWait {
        tokio::time::timeout(Duration::from_secs(1), fallbacks.recv())
            .await
            .expect("priority maintenance fallback wait should start promptly")
            .expect("priority maintenance fallback channel should remain open")
    }

    async fn wait_for_priority_maintenance_tasks(pool: &ConnectionPool) {
        for _ in 0..1_000 {
            let all_finished = pool
                .background_shutdown
                .lock()
                .tasks
                .as_ref()
                .is_none_or(|tasks| {
                    tasks
                        .priority_maintenance
                        .iter()
                        .all(JoinHandle::is_finished)
                });
            if all_finished {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("priority maintenance tasks should finish promptly");
    }

    async fn wait_for_active_count(pool: &ConnectionPool, expected: usize) {
        for _ in 0..1_000 {
            if pool.active_count() == expected {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("pool should reach active count {expected} promptly");
    }

    async fn wait_for_shutdown_coordinator(pool: &ConnectionPool) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if pool.background_shutdown.lock().coordinator_started {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("shutdown coordinator should start promptly");
    }

    async fn replace_health_task(pool: &ConnectionPool, replacement: JoinHandle<()>) {
        let original = {
            let mut shutdown = pool.background_shutdown.lock();
            let tasks = shutdown
                .tasks
                .as_mut()
                .expect("test health task must be replaced before coordinator start");
            std::mem::replace(&mut tasks.health, replacement)
        };
        original.abort();
        let error = original
            .await
            .expect_err("the original health task should be cancelled");
        assert!(error.is_cancelled());
    }

    async fn replace_health_with_blocking_task(
        pool: &ConnectionPool,
    ) -> (oneshot::Receiver<()>, std_mpsc::Sender<()>) {
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = std_mpsc::channel();
        let task = tokio::task::spawn_blocking(move || {
            let _ = started_tx.send(());
            let _ = release_rx.recv();
        });
        replace_health_task(pool, task).await;
        (started_rx, release_tx)
    }

    async fn connected_halves_with_peer() -> ((TcpSink, TcpRecvStream), tokio::net::TcpStream) {
        let listener = tokio::net::TcpListener::bind(test_addr(0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (halves, accepted) = tokio::join!(
            TcpTransport::connect(TcpConfig::default(), addr),
            listener.accept()
        );
        (halves.unwrap(), accepted.unwrap().0)
    }

    async fn connected_halves() -> (TcpSink, TcpRecvStream) {
        let (halves, mut peer) = connected_halves_with_peer().await;
        // Keep synthetic transport peers clean until their client half closes.
        tokio::spawn(async move {
            let mut buffer = [0_u8; 64];
            while peer.read(&mut buffer).await.unwrap_or(0) != 0 {}
        });
        halves
    }

    async fn idle_entry(addr: SocketAddr, last_used: Instant) -> PoolEntry {
        let (sink, stream) = connected_halves().await;
        PoolEntry {
            addr,
            sink,
            stream,
            last_used,
            is_priority: false,
        }
    }

    async fn idle_entry_with_peer(
        addr: SocketAddr,
        last_used: Instant,
        is_priority: bool,
    ) -> (PoolEntry, tokio::net::TcpStream) {
        let ((sink, stream), peer) = connected_halves_with_peer().await;
        (
            PoolEntry {
                addr,
                sink,
                stream,
                last_used,
                is_priority,
            },
            peer,
        )
    }

    async fn wait_for_entry_observation(
        mut entry: PoolEntry,
        expected: TcpIdleObservation,
    ) -> PoolEntry {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let (next, observation) = match inspect_idle_entry(entry) {
                    IdleEntryInspection::Retain(next) => {
                        (next, TcpIdleObservation::NoAdverseSignal)
                    }
                    IdleEntryInspection::Retire(next, observation) => (next, observation),
                };
                entry = next;
                if observation == expected {
                    return entry;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("idle socket state should become immediately observable")
    }

    fn response_adu(transaction_id: u16) -> [u8; 12] {
        let [high, low] = transaction_id.to_be_bytes();
        [
            high, low, 0x00, 0x00, 0x00, 0x06, 0xff, 0x03, 0x00, 0x00, 0x00, 0x01,
        ]
    }

    fn spawn_pending_get(
        pool: Arc<ConnectionPool>,
        addr: SocketAddr,
    ) -> (GetTask, oneshot::Receiver<()>) {
        let (entered_tx, entered_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            pool.get_with_connector(addr, move || async move {
                entered_tx
                    .send(())
                    .expect("pending connector entry receiver should remain live");
                pending::<Result<(TcpSink, TcpRecvStream), TransportError>>().await
            })
            .await
        });
        (task, entered_rx)
    }

    fn spawn_pending_timed_get(
        pool: Arc<ConnectionPool>,
        addr: SocketAddr,
        timeout: Duration,
    ) -> (GetTask, oneshot::Receiver<()>) {
        let (entered_tx, entered_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            pool.get_with_acquisition_timeout_and_connector(addr, timeout, move || async move {
                entered_tx
                    .send(())
                    .expect("pending connector entry receiver should remain live");
                pending::<Result<(TcpSink, TcpRecvStream), TransportError>>().await
            })
            .await
        });
        (task, entered_rx)
    }

    async fn acquired_connection(pool: &ConnectionPool, addr: SocketAddr) -> PooledConnection {
        pool.get_with_connector(addr, || async { Ok(connected_halves().await) })
            .await
            .expect("test connector should establish a connection")
    }

    async fn abort_pending(task: GetTask) {
        // Repeated cancellation requests must still drop one reservation once.
        task.abort();
        task.abort();
        let error = task
            .await
            .expect_err("pending acquisition should be cancelled");
        assert!(error.is_cancelled());
    }

    #[tokio::test]
    async fn priority_maintenance_sender_closure_is_a_sticky_stop() {
        let (stop, mut stopped) = watch::channel(false);
        drop(stop);

        assert!(priority_maintenance_should_stop(&stopped));
        tokio::time::timeout(
            Duration::from_secs(1),
            wait_for_priority_maintenance_stop(&mut stopped),
        )
        .await
        .expect("closed maintenance-stop sender should be observed promptly");
    }

    #[tokio::test]
    async fn shutdown_and_wait_empty_pool_is_sticky_and_prompt() {
        let pool = ConnectionPool::new(test_config(1));

        tokio::time::timeout(Duration::from_secs(1), pool.shutdown_and_wait())
            .await
            .expect("empty pool background tasks should join promptly");
        tokio::time::timeout(Duration::from_secs(1), async {
            pool.shutdown_and_wait().await;
            pool.shutdown_and_wait().await;
        })
        .await
        .expect("repeated calls should observe sticky completion promptly");

        let shutdown = pool.background_shutdown.lock();
        assert!(shutdown.coordinator_started);
        assert!(shutdown.tasks.is_none());
        assert!(*shutdown.completed.borrow());
    }

    #[tokio::test]
    async fn shutdown_and_wait_hard_aborts_and_joins_health() {
        let pool = ConnectionPool::new(test_config(1));
        let (started_tx, started_rx) = oneshot::channel();
        let (dropped_tx, mut dropped_rx) = oneshot::channel();
        let health = tokio::spawn(async move {
            let _dropped = DropSignal(Some(dropped_tx));
            started_tx.send(()).unwrap();
            pending::<()>().await;
        });
        replace_health_task(&pool, health).await;
        started_rx.await.unwrap();

        tokio::time::timeout(Duration::from_secs(1), pool.shutdown_and_wait())
            .await
            .expect("hard-aborted health task should join promptly");

        assert_eq!(dropped_rx.try_recv(), Ok(()));
        assert!(*pool.background_shutdown.lock().completed.borrow());
    }

    #[tokio::test]
    async fn shutdown_and_wait_joins_pending_priority_maintenance_and_releases_reservation() {
        let addr = test_addr(15_220);
        let config = replenishment_config(vec![crate::PriorityDevice {
            addr,
            max_connections: 1,
        }]);
        let (pool, mut attempts, mut retries, mut fallbacks) =
            pool_with_gated_priority_maintenance(config);
        let attempt = next_priority_maintenance_attempt(&mut attempts).await;
        assert_eq!(pool.active_count(), 1);

        tokio::time::timeout(Duration::from_secs(1), pool.shutdown_and_wait())
            .await
            .expect("maintenance cooperative stop and join should complete promptly");

        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
        assert!(!pool.inner.lock().active_priority.contains_key(&addr));
        assert!(
            attempt
                .complete
                .send(GatedConnectCompletion::SyntheticSuccess)
                .is_err()
        );
        assert!(matches!(
            retries.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
        assert!(matches!(
            fallbacks.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
    }

    #[tokio::test]
    async fn shutdown_and_wait_joins_priority_maintenance_parked_in_backoff() {
        let addr = test_addr(15_221);
        let config = replenishment_config(vec![crate::PriorityDevice {
            addr,
            max_connections: 1,
        }]);
        let (pool, mut attempts, mut retries, _fallbacks) =
            pool_with_gated_priority_maintenance(config);
        next_priority_maintenance_attempt(&mut attempts)
            .await
            .fail();
        let retry = next_retry_wait(&mut retries).await;
        assert_eq!(pool.active_count(), 0);
        assert!(!pool.inner.lock().active_priority.contains_key(&addr));

        tokio::time::timeout(Duration::from_secs(1), pool.shutdown_and_wait())
            .await
            .expect("parked backoff task should stop and join promptly");

        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
        assert!(retry.complete.send(()).is_err());
        assert!(matches!(
            attempts.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
        assert!(matches!(
            retries.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
    }

    #[tokio::test]
    async fn shutdown_and_wait_joins_priority_maintenance_parked_in_fallback() {
        let addr = test_addr(15_226);
        let config = replenishment_config(vec![crate::PriorityDevice {
            addr,
            max_connections: 1,
        }]);
        let (pool, mut attempts, mut retries, mut fallbacks) =
            pool_with_gated_priority_maintenance(config);
        next_priority_maintenance_attempt(&mut attempts)
            .await
            .succeed();
        let fallback = next_fallback_wait(&mut fallbacks).await;
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 1);

        tokio::time::timeout(Duration::from_secs(1), pool.shutdown_and_wait())
            .await
            .expect("parked fallback task should stop and join promptly");

        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
        assert!(fallback.complete.send(()).is_err());
        assert!(matches!(
            attempts.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
        assert!(matches!(
            retries.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
        assert!(matches!(
            fallbacks.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_first_shutdown_waiter_does_not_lose_join_ownership() {
        let pool = ConnectionPool::new(test_config(1));
        let (blocking_started, release_blocking) = replace_health_with_blocking_task(&pool).await;
        blocking_started.await.unwrap();
        let pool = Arc::new(pool);

        let first_pool = Arc::clone(&pool);
        let first = tokio::spawn(async move { first_pool.shutdown_and_wait().await });
        wait_for_shutdown_coordinator(&pool).await;
        assert!(!first.is_finished());
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());
        assert!(!*pool.background_shutdown.lock().completed.borrow());

        let second_pool = Arc::clone(&pool);
        let second = tokio::spawn(async move { second_pool.shutdown_and_wait().await });
        tokio::task::yield_now().await;
        assert!(!second.is_finished());
        release_blocking.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), second)
            .await
            .expect("later waiter should observe coordinator completion")
            .expect("later waiter task should not fail");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_shutdown_waiters_share_completion_and_one_may_cancel() {
        let pool = ConnectionPool::new(test_config(1));
        let (blocking_started, release_blocking) = replace_health_with_blocking_task(&pool).await;
        blocking_started.await.unwrap();
        let pool = Arc::new(pool);

        let mut waiters = Vec::new();
        for _ in 0..3 {
            let waiter_pool = Arc::clone(&pool);
            waiters.push(tokio::spawn(async move {
                waiter_pool.shutdown_and_wait().await;
            }));
        }
        wait_for_shutdown_coordinator(&pool).await;
        waiters[0].abort();
        assert!(waiters.remove(0).await.unwrap_err().is_cancelled());
        assert!(waiters.iter().all(|waiter| !waiter.is_finished()));

        release_blocking.send(()).unwrap();
        for waiter in waiters {
            tokio::time::timeout(Duration::from_secs(1), waiter)
                .await
                .expect("shared completion should wake every waiter")
                .expect("shutdown waiter task should not fail");
        }
        assert!(*pool.background_shutdown.lock().completed.borrow());
    }

    #[tokio::test]
    async fn synchronous_shutdown_then_async_wait_joins_already_stopping_tasks() {
        let addr = test_addr(15_222);
        let config = pre_connect_config(vec![crate::PriorityDevice {
            addr,
            max_connections: 1,
        }]);
        let (pool, mut attempts, _retries, _fallbacks) =
            pool_with_gated_priority_maintenance(config);
        let attempt = next_priority_maintenance_attempt(&mut attempts).await;
        assert_eq!(pool.active_count(), 1);

        pool.shutdown();
        pool.shutdown();
        wait_for_priority_maintenance_tasks(&pool).await;
        {
            let shutdown = pool.background_shutdown.lock();
            assert!(!shutdown.coordinator_started);
            assert!(shutdown.tasks.is_some());
            assert!(
                shutdown
                    .tasks
                    .as_ref()
                    .unwrap()
                    .priority_maintenance
                    .iter()
                    .all(JoinHandle::is_finished)
            );
        }
        assert_eq!(pool.active_count(), 0);

        pool.shutdown_and_wait().await;
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
        assert!(
            attempt
                .complete
                .send(GatedConnectCompletion::SyntheticSuccess)
                .is_err()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn synchronous_shutdown_sticky_stop_before_first_poll_starts_no_activity() {
        let addr = test_addr(15_227);
        let config = replenishment_config(vec![crate::PriorityDevice {
            addr,
            max_connections: 1,
        }]);
        let (pool, mut attempts, mut retries, mut fallbacks) =
            pool_with_gated_priority_maintenance(config);

        pool.shutdown();
        pool.shutdown_and_wait().await;

        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
        assert!(matches!(
            attempts.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
        assert!(matches!(
            retries.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
        assert!(matches!(
            fallbacks.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
    }

    #[tokio::test]
    async fn concurrent_sync_shutdown_and_async_wait_release_pre_connect_once() {
        let addr = test_addr(15_223);
        let config = pre_connect_config(vec![crate::PriorityDevice {
            addr,
            max_connections: 2,
        }]);
        let (pool, mut attempts, _retries, _fallbacks) =
            pool_with_gated_priority_maintenance(config);
        let attempt = next_priority_maintenance_attempt(&mut attempts).await;
        let demand = acquired_connection(&pool, addr).await;
        assert_eq!(pool.active_count(), 2);
        let pool = Arc::new(pool);
        let barrier = Arc::new(Barrier::new(3));

        let sync_pool = Arc::clone(&pool);
        let sync_barrier = Arc::clone(&barrier);
        let synchronous = tokio::spawn(async move {
            sync_barrier.wait().await;
            sync_pool.shutdown();
            sync_pool.shutdown();
        });
        let async_pool = Arc::clone(&pool);
        let async_barrier = Arc::clone(&barrier);
        let asynchronous = tokio::spawn(async move {
            async_barrier.wait().await;
            async_pool.shutdown_and_wait().await;
            async_pool.shutdown_and_wait().await;
        });
        barrier.wait().await;
        synchronous.await.unwrap();
        asynchronous.await.unwrap();

        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.inner.lock().active_priority.get(&addr), Some(&1));
        assert!(
            attempt
                .complete
                .send(GatedConnectCompletion::SyntheticSuccess)
                .is_err()
        );
        drop(demand);
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn background_task_panic_warns_and_does_not_strand_other_joins() {
        let _tracing_test = TRACING_TEST_LOCK.lock().await;
        let addr = test_addr(15_228);
        let config = pre_connect_config(vec![crate::PriorityDevice {
            addr,
            max_connections: 1,
        }]);
        let (pool, mut attempts, _retries, _fallbacks) =
            pool_with_gated_priority_maintenance(config);
        let maintenance = next_priority_maintenance_attempt(&mut attempts).await;
        assert_eq!(pool.active_count(), 1);

        let panicked = tokio::spawn(async { panic!("synthetic pool task panic") });
        while !panicked.is_finished() {
            tokio::task::yield_now().await;
        }
        replace_health_task(&pool, panicked).await;

        let capture = ShutdownWarningCapture::default();
        let dispatch = tracing::Dispatch::new(registry().with(capture.clone()));
        let _default = dispatcher::set_default(&dispatch);
        pool.shutdown_and_wait().await;

        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
        assert!(
            maintenance
                .complete
                .send(GatedConnectCompletion::SyntheticSuccess)
                .is_err()
        );
        assert_eq!(
            capture.events.lock().unwrap().as_slice(),
            [CapturedShutdownWarning {
                message: "pool_background_task_join_failed".to_owned(),
                unexpected_join_errors: 1,
                task_count: 2,
            }]
        );
    }

    #[tokio::test]
    async fn shutdown_task_quiescence_excludes_checked_out_raw_lease() {
        let pool = ConnectionPool::new(test_config(1));
        let lease = acquired_connection(&pool, test_addr(15_224)).await;
        assert_eq!(pool.active_count(), 1);

        pool.shutdown_and_wait().await;

        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.idle_count(), 0);
        drop(lease);
        assert_eq!(pool.active_count(), 0);
    }

    #[tokio::test]
    async fn shutdown_task_quiescence_excludes_pending_demand_connector() {
        let pool = Arc::new(ConnectionPool::new(test_config(1)));
        let (demand, entered) = spawn_pending_get(Arc::clone(&pool), test_addr(15_225));
        entered.await.unwrap();
        assert_eq!(pool.active_count(), 1);

        tokio::time::timeout(Duration::from_secs(1), pool.shutdown_and_wait())
            .await
            .expect("caller-owned demand connector must not delay pool task quiescence");

        assert!(!demand.is_finished());
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.idle_count(), 0);
        abort_pending(demand).await;
        assert_eq!(pool.active_count(), 0);
    }

    #[test]
    fn synchronous_shutdown_and_drop_after_runtime_teardown_do_not_panic() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let pool = runtime.block_on(async { ConnectionPool::new(test_config(1)) });
        drop(runtime);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pool.shutdown();
            pool.shutdown();
            drop(pool);
        }));
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn replenishment_starts_initial_warmup_when_pre_connect_is_disabled() {
        let addr = test_addr(15_230);
        let config = replenishment_config(vec![crate::PriorityDevice {
            addr,
            max_connections: 1,
        }]);
        assert!(!config.pre_connect);
        assert!(config.priority_replenishment);
        let (pool, mut attempts, _retries, mut fallbacks) =
            pool_with_gated_priority_maintenance(config);

        let attempt = next_priority_maintenance_attempt(&mut attempts).await;
        assert_eq!(attempt.addr, addr);
        assert_eq!(pool.active_count(), 1);
        attempt.succeed();
        let fallback = next_fallback_wait(&mut fallbacks).await;

        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.inner.lock().priority_idle_count(addr), 1);
        pool.shutdown_and_wait().await;
        assert!(fallback.complete.send(()).is_err());
    }

    #[tokio::test]
    async fn standing_checkout_wakes_and_restores_one_idle_below_device_cap() {
        let addr = test_addr(15_231);
        let config = replenishment_config(vec![crate::PriorityDevice {
            addr,
            max_connections: 2,
        }]);
        let (pool, mut attempts, _retries, mut fallbacks) =
            pool_with_gated_priority_maintenance(config);

        next_priority_maintenance_attempt(&mut attempts)
            .await
            .succeed();
        let initial_fallback = next_fallback_wait(&mut fallbacks).await;
        assert_eq!(pool.inner.lock().priority_idle_count(addr), 1);

        let checked_out = pool
            .get_with_connector(addr, || async { Err(TransportError::Disconnected) })
            .await
            .expect("clean maintenance entry should be reused");
        let replenishment = next_priority_maintenance_attempt(&mut attempts).await;
        assert!(initial_fallback.complete.send(()).is_err());
        assert_eq!(pool.active_count(), 2);
        assert_eq!(pool.idle_count(), 0);

        replenishment.succeed();
        let fallback = next_fallback_wait(&mut fallbacks).await;
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.inner.lock().priority_idle_count(addr), 1);
        assert_eq!(pool.inner.lock().priority_total(addr), 2);

        drop(checked_out);
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.inner.lock().priority_idle_count(addr), 1);
        pool.shutdown_and_wait().await;
        assert!(fallback.complete.send(()).is_err());
    }

    #[tokio::test]
    async fn standing_waits_at_cap_then_invalidation_and_raw_drop_wake_replenishment() {
        let addr = test_addr(15_232);
        let config = replenishment_config(vec![crate::PriorityDevice {
            addr,
            max_connections: 1,
        }]);
        let (pool, mut attempts, _retries, mut fallbacks) =
            pool_with_gated_priority_maintenance(config);

        next_priority_maintenance_attempt(&mut attempts)
            .await
            .succeed();
        let idle_fallback = next_fallback_wait(&mut fallbacks).await;
        let mut invalidated = pool
            .get_with_connector(addr, || async { Err(TransportError::Disconnected) })
            .await
            .expect("initial idle should be reusable");
        let active_fallback = next_fallback_wait(&mut fallbacks).await;
        assert!(idle_fallback.complete.send(()).is_err());
        assert!(matches!(
            attempts.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        assert_eq!(pool.active_count(), 1);

        invalidated.invalidate(crate::LeaseInvalidationReason::Transport);
        let after_invalidation = next_priority_maintenance_attempt(&mut attempts).await;
        assert!(active_fallback.complete.send(()).is_err());
        assert_eq!(pool.active_count(), 1);
        after_invalidation.succeed();
        let returned_fallback = next_fallback_wait(&mut fallbacks).await;

        let raw = pool
            .get_with_connector(addr, || async { Err(TransportError::Disconnected) })
            .await
            .expect("replacement idle should be reusable");
        let raw_active_fallback = next_fallback_wait(&mut fallbacks).await;
        assert!(returned_fallback.complete.send(()).is_err());
        assert!(matches!(
            attempts.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        drop(raw);
        let after_raw_drop = next_priority_maintenance_attempt(&mut attempts).await;
        assert!(raw_active_fallback.complete.send(()).is_err());
        assert_eq!(pool.active_count(), 1);

        pool.shutdown_and_wait().await;
        assert_eq!(pool.active_count(), 0);
        assert!(
            after_raw_drop
                .complete
                .send(GatedConnectCompletion::SyntheticSuccess)
                .is_err()
        );
    }

    #[tokio::test]
    async fn demand_connect_failure_releases_priority_capacity_for_maintenance() {
        let addr = test_addr(15_233);
        let config = replenishment_config(vec![crate::PriorityDevice {
            addr,
            max_connections: 1,
        }]);
        let (pool, mut attempts, _retries, _fallbacks) =
            pool_with_gated_priority_maintenance(config);

        // This ready connector is polled to completion without yielding, so the
        // demand path deterministically reserves and releases before the spawned
        // maintenance task can run.
        let result = pool
            .get_with_connector(addr, || async { Err(TransportError::Disconnected) })
            .await;
        assert!(matches!(result, Err(PoolError::ConnectionFailed(_))));
        assert_eq!(pool.active_count(), 0);

        let maintenance = next_priority_maintenance_attempt(&mut attempts).await;
        assert_eq!(pool.active_count(), 1);
        pool.shutdown_and_wait().await;
        assert_eq!(pool.active_count(), 0);
        assert!(
            maintenance
                .complete
                .send(GatedConnectCompletion::SyntheticSuccess)
                .is_err()
        );
    }

    #[tokio::test]
    async fn maintenance_failure_backoff_resets_after_successful_establishment() {
        let addr = test_addr(15_234);
        let config = replenishment_config(vec![crate::PriorityDevice {
            addr,
            max_connections: 1,
        }]);
        let (pool, mut attempts, mut retries, mut fallbacks) =
            pool_with_gated_priority_maintenance(config);

        next_priority_maintenance_attempt(&mut attempts)
            .await
            .fail();
        let first_retry = next_retry_wait(&mut retries).await;
        assert_eq!(first_retry.delay, Duration::from_millis(10));
        first_retry.resume();
        next_priority_maintenance_attempt(&mut attempts)
            .await
            .fail();
        let second_retry = next_retry_wait(&mut retries).await;
        assert_eq!(second_retry.delay, Duration::from_millis(20));
        second_retry.resume();

        next_priority_maintenance_attempt(&mut attempts)
            .await
            .succeed();
        let idle_fallback = next_fallback_wait(&mut fallbacks).await;
        let mut checked_out = pool
            .get_with_connector(addr, || async { Err(TransportError::Disconnected) })
            .await
            .expect("successful maintenance connection should be reusable");
        let active_fallback = next_fallback_wait(&mut fallbacks).await;
        assert!(idle_fallback.complete.send(()).is_err());
        checked_out.invalidate(crate::LeaseInvalidationReason::Transport);

        next_priority_maintenance_attempt(&mut attempts)
            .await
            .fail();
        assert!(active_fallback.complete.send(()).is_err());
        let reset_retry = next_retry_wait(&mut retries).await;
        assert_eq!(reset_retry.delay, Duration::from_millis(10));
        pool.shutdown_and_wait().await;
        assert!(reset_retry.complete.send(()).is_err());
    }

    #[tokio::test]
    async fn pending_maintenance_success_racing_idle_insertion_drops_redundant_transport() {
        let addr = test_addr(15_235);
        let config = replenishment_config(vec![crate::PriorityDevice {
            addr,
            max_connections: 2,
        }]);
        let (pool, mut attempts, _retries, mut fallbacks) =
            pool_with_gated_priority_maintenance(config);
        let pending = next_priority_maintenance_attempt(&mut attempts).await;

        let (existing, _existing_peer) = idle_entry_with_peer(addr, Instant::now(), true).await;
        pool.inner.lock().idle.push(existing);
        pool.capacity_changed.notify_waiters();

        let (redundant_halves, mut redundant_peer) = connected_halves_with_peer().await;
        let (redundant_dropped_tx, redundant_dropped_rx) = oneshot::channel();
        tokio::spawn(async move {
            let mut byte = [0_u8; 1];
            let result = redundant_peer.read(&mut byte).await;
            let _ = redundant_dropped_tx.send(result);
        });
        pending.succeed_with(redundant_halves);
        let fallback = next_fallback_wait(&mut fallbacks).await;

        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.inner.lock().priority_idle_count(addr), 1);
        assert_eq!(pool.inner.lock().priority_total(addr), 1);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), redundant_dropped_rx)
                .await
                .expect("redundant transport should close promptly")
                .expect("redundant transport observer should remain live")
                .expect("redundant peer read should complete"),
            0
        );
        assert!(matches!(
            attempts.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        pool.shutdown_and_wait().await;
        assert!(fallback.complete.send(()).is_err());
    }

    #[tokio::test]
    async fn passive_priority_retirement_wakes_standing_maintenance() {
        let addr = test_addr(15_236);
        let config = replenishment_config(vec![crate::PriorityDevice {
            addr,
            max_connections: 1,
        }]);
        let (pool, mut attempts, mut retries, mut fallbacks) =
            pool_with_gated_priority_maintenance(config);
        let pending = next_priority_maintenance_attempt(&mut attempts).await;

        let (adverse, peer) = idle_entry_with_peer(addr, Instant::now(), true).await;
        drop(peer);
        let adverse = wait_for_entry_observation(adverse, TcpIdleObservation::PeerClosed).await;
        pool.inner.lock().idle.push(adverse);
        pending.fail();
        let retry = next_retry_wait(&mut retries).await;
        retry.resume();
        let fallback = next_fallback_wait(&mut fallbacks).await;
        assert_eq!(pool.inner.lock().priority_idle_count(addr), 1);

        assert!(health::run_health_check(
            &pool.inner,
            &pool.capacity_changed,
            Instant::now(),
            Duration::from_hours(1),
        ));
        let replacement = next_priority_maintenance_attempt(&mut attempts).await;
        assert!(fallback.complete.send(()).is_err());
        assert_eq!(pool.idle_count(), 0);
        assert_eq!(pool.active_count(), 1);

        pool.shutdown_and_wait().await;
        assert!(
            replacement
                .complete
                .send(GatedConnectCompletion::SyntheticSuccess)
                .is_err()
        );
    }

    #[tokio::test]
    async fn standing_fallback_recovers_an_intentionally_omitted_notification() {
        let addr = test_addr(15_237);
        let config = replenishment_config(vec![crate::PriorityDevice {
            addr,
            max_connections: 1,
        }]);
        let expected_interval = config.health_check_interval;
        let (pool, mut attempts, _retries, mut fallbacks) =
            pool_with_gated_priority_maintenance(config);

        next_priority_maintenance_attempt(&mut attempts)
            .await
            .succeed();
        let fallback = next_fallback_wait(&mut fallbacks).await;
        assert_eq!(fallback.delay, expected_interval);
        let removed = pool
            .inner
            .lock()
            .idle
            .pop()
            .expect("initial maintenance entry should exist");
        drop(removed);
        assert!(matches!(
            attempts.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        // Deliberately omit capacity_changed.notify_waiters(): only the fresh
        // periodic safety fallback can drive this reevaluation.
        fallback.resume();
        let replacement = next_priority_maintenance_attempt(&mut attempts).await;
        assert_eq!(pool.active_count(), 1);
        pool.shutdown_and_wait().await;
        assert!(
            replacement
                .complete
                .send(GatedConnectCompletion::SyntheticSuccess)
                .is_err()
        );
    }

    #[tokio::test]
    async fn pending_pre_connect_reserves_before_fail_fast_demand() {
        let addr = test_addr(15_200);
        let config = pre_connect_config(vec![crate::PriorityDevice {
            addr,
            max_connections: 1,
        }]);
        let (pool, mut attempts, _retries, _fallbacks) =
            pool_with_gated_priority_maintenance(config);
        let attempt = next_priority_maintenance_attempt(&mut attempts).await;
        assert_eq!(attempt.addr, addr);
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.inner.lock().priority_total(addr), 1);

        let demand_connector_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&demand_connector_calls);
        let result = pool
            .get_with_connector(addr, move || {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Err(TransportError::Disconnected) }
            })
            .await;

        assert!(matches!(result, Err(PoolError::Exhausted)));
        assert_eq!(demand_connector_calls.load(Ordering::SeqCst), 0);
        assert_eq!(pool.active_count(), 1);

        attempt.succeed();
        wait_for_priority_maintenance_tasks(&pool).await;
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 1);
        assert_eq!(pool.inner.lock().priority_total(addr), 1);
        assert!(matches!(
            attempts.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
    }

    #[tokio::test]
    async fn failed_pre_connect_releases_for_demand_and_retry_exits_when_full() {
        let addr = test_addr(15_201);
        let demand_halves = connected_halves().await;
        let config = pre_connect_config(vec![crate::PriorityDevice {
            addr,
            max_connections: 1,
        }]);
        let (pool, mut attempts, mut retries, _fallbacks) =
            pool_with_gated_priority_maintenance(config);
        let pool = Arc::new(pool);
        let first_attempt = next_priority_maintenance_attempt(&mut attempts).await;
        assert_eq!(pool.active_count(), 1);

        let (demand_started_tx, demand_started_rx) = oneshot::channel();
        let (demand_connector_tx, demand_connector_rx) = oneshot::channel();
        let demand_pool = Arc::clone(&pool);
        let demand = tokio::spawn(async move {
            demand_started_tx
                .send(())
                .expect("demand start receiver should remain live");
            demand_pool
                .get_with_acquisition_timeout_and_connector(
                    addr,
                    Duration::from_secs(5),
                    move || async move {
                        demand_connector_tx
                            .send(())
                            .expect("demand connector receiver should remain live");
                        Ok(demand_halves)
                    },
                )
                .await
        });
        demand_started_rx.await.unwrap();
        tokio::task::yield_now().await;
        assert!(!demand.is_finished());

        first_attempt.fail();
        let retry = next_retry_wait(&mut retries).await;
        assert_eq!(retry.delay, Duration::from_millis(10));
        demand_connector_rx
            .await
            .expect("failure rollback should wake demand to reserve");
        let connection = demand
            .await
            .expect("demand task should complete")
            .expect("released priority capacity should be reservable");
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.idle_count(), 0);

        // Release the deterministic backoff gate. Demand now fills the budget,
        // so the retry exits without starting a second connector.
        retry.resume();
        wait_for_priority_maintenance_tasks(&pool).await;
        assert!(matches!(
            attempts.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
        assert_eq!(pool.active_count(), 1);

        drop(connection);
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
    }

    #[tokio::test]
    async fn legacy_one_shot_warms_for_reuse_and_does_not_replenish_retirement() {
        let addr = test_addr(15_202);
        let config = pre_connect_config(vec![crate::PriorityDevice {
            addr,
            max_connections: 1,
        }]);
        assert!(!config.priority_replenishment);
        let (pool, mut attempts, _retries, _fallbacks) =
            pool_with_gated_priority_maintenance(config);
        let pool = Arc::new(pool);
        let attempt = next_priority_maintenance_attempt(&mut attempts).await;

        let demand_connector_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&demand_connector_calls);
        let (demand_started_tx, demand_started_rx) = oneshot::channel();
        let demand_pool = Arc::clone(&pool);
        let waiter = tokio::spawn(async move {
            demand_started_tx
                .send(())
                .expect("demand start receiver should remain live");
            demand_pool
                .get_with_acquisition_timeout_and_connector(
                    addr,
                    Duration::from_secs(5),
                    move || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        async { Err(TransportError::Disconnected) }
                    },
                )
                .await
        });
        demand_started_rx.await.unwrap();
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        assert_eq!(demand_connector_calls.load(Ordering::SeqCst), 0);

        attempt.succeed();
        let connection = waiter
            .await
            .expect("waiter task should complete")
            .expect("waiter should claim the newly idle transport");
        wait_for_priority_maintenance_tasks(&pool).await;
        assert_eq!(connection.addr(), addr);
        assert_eq!(demand_connector_calls.load(Ordering::SeqCst), 0);
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.idle_count(), 0);
        assert!(matches!(
            attempts.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));

        // Raw retirement does not replenish the one-time warm-up entry.
        drop(connection);
        tokio::task::yield_now().await;
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
    }

    #[tokio::test]
    async fn shutdown_cooperatively_stops_pending_pre_connect_without_idle_resurrection() {
        let addr = test_addr(15_203);
        let demand_halves = connected_halves().await;
        let config = pre_connect_config(vec![crate::PriorityDevice {
            addr,
            max_connections: 2,
        }]);
        let (pool, mut attempts, _retries, _fallbacks) =
            pool_with_gated_priority_maintenance(config);
        let attempt = next_priority_maintenance_attempt(&mut attempts).await;
        let demand = pool
            .get_with_connector(addr, move || async move { Ok(demand_halves) })
            .await
            .expect("the second device charge should remain available to demand");
        assert_eq!(pool.active_count(), 2);

        pool.shutdown();
        pool.shutdown();
        let _ = attempt
            .complete
            .send(GatedConnectCompletion::SyntheticSuccess);
        wait_for_priority_maintenance_tasks(&pool).await;
        wait_for_active_count(&pool, 1).await;

        // The sibling demand charge proves cancellation released exactly the
        // pre-connect charge once; shutdown cannot insert the completed entry.
        assert_eq!(pool.inner.lock().active_priority.get(&addr), Some(&1));
        assert_eq!(pool.idle_count(), 0);
        drop(demand);
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
    }

    #[tokio::test]
    async fn completed_pre_connect_after_shutdown_releases_without_idle() {
        let addr = test_addr(15_208);
        let mut config = test_config(1);
        config.priority_devices = vec![crate::PriorityDevice {
            addr,
            max_connections: 1,
        }];
        let pool = ConnectionPool::new(config);
        let reservation = {
            let mut inner = pool.inner.lock();
            *inner.active_priority.entry(addr).or_insert(0) += 1;
            PendingReservation::new(
                Arc::clone(&pool.inner),
                Arc::clone(&pool.capacity_changed),
                addr,
                true,
                None,
            )
        };
        let (sink, stream) = connected_halves().await;
        assert_eq!(pool.active_count(), 1);

        pool.shutdown();
        reservation.commit_priority_idle_if_needed(PoolEntry {
            addr,
            sink,
            stream,
            last_used: Instant::now(),
            is_priority: true,
        });

        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
        assert!(!pool.inner.lock().active_priority.contains_key(&addr));
    }

    #[tokio::test]
    async fn independent_priority_addresses_reserve_independent_pre_connect_budgets() {
        let first_addr = test_addr(15_204);
        let second_addr = test_addr(15_205);
        let config = pre_connect_config(vec![
            crate::PriorityDevice {
                addr: first_addr,
                max_connections: 1,
            },
            crate::PriorityDevice {
                addr: second_addr,
                max_connections: 1,
            },
        ]);
        let (pool, mut attempts, _retries, _fallbacks) =
            pool_with_gated_priority_maintenance(config);
        let first = next_priority_maintenance_attempt(&mut attempts).await;
        let second = next_priority_maintenance_attempt(&mut attempts).await;

        assert_eq!(
            HashSet::from([first.addr, second.addr]),
            HashSet::from([first_addr, second_addr])
        );
        {
            let inner = pool.inner.lock();
            assert_eq!(inner.active_priority.get(&first_addr), Some(&1));
            assert_eq!(inner.active_priority.get(&second_addr), Some(&1));
            assert_eq!(inner.active_total(), 2);
        }
        assert!(matches!(
            attempts.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        pool.shutdown();
        drop(first);
        drop(second);
        wait_for_priority_maintenance_tasks(&pool).await;
        wait_for_active_count(&pool, 0).await;
    }

    #[tokio::test]
    async fn both_flags_and_duplicate_addresses_spawn_one_first_cap_maintenance_task() {
        let addr = test_addr(15_206);
        let mut config = pre_connect_config(vec![
            crate::PriorityDevice {
                addr,
                max_connections: 1,
            },
            crate::PriorityDevice {
                addr,
                max_connections: 3,
            },
        ]);
        config.priority_replenishment = true;
        let (pool, mut attempts, _retries, _fallbacks) =
            pool_with_gated_priority_maintenance(config);
        let attempt = next_priority_maintenance_attempt(&mut attempts).await;
        assert_eq!(attempt.addr, addr);
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(matches!(
            attempts.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        assert_eq!(
            pool.background_shutdown
                .lock()
                .tasks
                .as_ref()
                .expect("coordinator has not started")
                .priority_maintenance
                .len(),
            1
        );
        assert_eq!(pool.active_count(), 1);
        let demand_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&demand_calls);
        assert!(matches!(
            pool.get_with_connector(addr, move || {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Err(TransportError::Disconnected) }
            })
            .await,
            Err(PoolError::Exhausted)
        ));
        assert_eq!(demand_calls.load(Ordering::SeqCst), 0);
        assert_eq!(pool.priority_cap(addr), 1);

        pool.shutdown();
        drop(attempt);
        wait_for_priority_maintenance_tasks(&pool).await;
        wait_for_active_count(&pool, 0).await;
    }

    #[tokio::test]
    async fn first_zero_priority_cap_spawns_no_maintenance_task_or_connector() {
        let addr = test_addr(15_207);
        let mut config = pre_connect_config(vec![
            crate::PriorityDevice {
                addr,
                max_connections: 0,
            },
            crate::PriorityDevice {
                addr,
                max_connections: 2,
            },
        ]);
        config.priority_replenishment = true;
        let (pool, mut attempts, _retries, _fallbacks) =
            pool_with_gated_priority_maintenance(config);
        wait_for_priority_maintenance_tasks(&pool).await;

        assert!(matches!(
            attempts.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
        assert_eq!(pool.inner.lock().priority_total(addr), 0);
        assert_eq!(
            pool.background_shutdown
                .lock()
                .tasks
                .as_ref()
                .expect("coordinator has not started")
                .priority_maintenance
                .len(),
            0
        );
    }

    #[test]
    fn idle_validation_labels_are_stable_and_bounded() {
        assert_eq!(IdleValidationTrigger::Checkout.as_str(), "checkout");
        assert_eq!(IdleValidationTrigger::HealthSweep.as_str(), "health_sweep");
        assert_eq!(
            idle_observation_reason(TcpIdleObservation::QueuedInput),
            "queued_input"
        );
        assert_eq!(
            idle_observation_reason(TcpIdleObservation::PeerClosed),
            "peer_closed"
        );
        assert_eq!(
            idle_observation_reason(TcpIdleObservation::SocketError(
                std::io::ErrorKind::ConnectionReset
            )),
            "socket_error"
        );
        assert_eq!(
            idle_observation_reason(TcpIdleObservation::MismatchedHalves),
            "mismatched_halves"
        );
    }

    #[tokio::test]
    async fn checkout_reuses_clean_idle_without_running_connector() {
        let addr = test_addr(15_090);
        let pool = ConnectionPool::new(test_config(1));
        let (entry, _peer) = idle_entry_with_peer(addr, Instant::now(), false).await;
        pool.inner.lock().idle.push(entry);

        let connection = pool
            .get_with_connector(addr, || async { Err(TransportError::Disconnected) })
            .await
            .expect("a clean idle entry should be reused");

        assert_eq!(connection.addr(), addr);
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.idle_count(), 0);
        drop(connection);
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
    }

    #[tokio::test]
    async fn queued_late_response_is_retired_before_next_borrower() {
        let addr = test_addr(15_091);
        let pool = ConnectionPool::new(test_config(1));
        let (entry, mut stale_peer) = idle_entry_with_peer(addr, Instant::now(), false).await;
        stale_peer.write_all(&response_adu(0x1234)).await.unwrap();
        let entry = wait_for_entry_observation(entry, TcpIdleObservation::QueuedInput).await;
        pool.inner.lock().idle.push(entry);

        let (replacement_halves, mut replacement_peer) = connected_halves_with_peer().await;
        let (connector_entered_tx, connector_entered_rx) = oneshot::channel();
        let mut connection = pool
            .get_with_connector(addr, move || async move {
                connector_entered_tx
                    .send(())
                    .expect("connector entry receiver should remain live");
                Ok(replacement_halves)
            })
            .await
            .expect("queued idle input should be replaced by a fresh connection");
        connector_entered_rx
            .await
            .expect("checkout should run the fresh connector");

        replacement_peer
            .write_all(&response_adu(0xbeef))
            .await
            .unwrap();
        let response = connection.stream().recv().await.unwrap();
        assert!(matches!(
            response.header,
            FrameHeader::Mbap(header) if header.transaction_id.get() == 0xbeef
        ));
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.idle_count(), 0);
    }

    #[tokio::test]
    async fn checkout_retires_observable_eof_and_connects_fresh() {
        let addr = test_addr(15_092);
        let pool = ConnectionPool::new(test_config(1));
        let (entry, stale_peer) = idle_entry_with_peer(addr, Instant::now(), false).await;
        drop(stale_peer);
        let entry = wait_for_entry_observation(entry, TcpIdleObservation::PeerClosed).await;
        pool.inner.lock().idle.push(entry);

        let replacement_halves = connected_halves().await;
        let (connector_entered_tx, connector_entered_rx) = oneshot::channel();
        let connection = pool
            .get_with_connector(addr, move || async move {
                connector_entered_tx
                    .send(())
                    .expect("connector entry receiver should remain live");
                Ok(replacement_halves)
            })
            .await
            .expect("closed idle connection should be replaced");
        connector_entered_rx
            .await
            .expect("EOF retirement should run the fresh connector");

        assert_eq!(connection.addr(), addr);
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.idle_count(), 0);
    }

    #[tokio::test]
    async fn invalid_same_address_entries_are_skipped_and_freed_capacity_wakes_waiter() {
        let addr = test_addr(15_093);
        let waiting_addr = test_addr(15_094);
        let pool = Arc::new(ConnectionPool::new(test_config(3)));
        let future_last_used = Instant::now() + Duration::from_mins(1);

        let (invalid_one, mut peer_one) = idle_entry_with_peer(addr, future_last_used, false).await;
        let (valid, _valid_peer) = idle_entry_with_peer(addr, future_last_used, false).await;
        let (invalid_two, mut peer_two) = idle_entry_with_peer(addr, future_last_used, false).await;
        peer_one.write_all(&[0x01]).await.unwrap();
        peer_two.write_all(&[0x02]).await.unwrap();
        let invalid_one =
            wait_for_entry_observation(invalid_one, TcpIdleObservation::QueuedInput).await;
        let invalid_two =
            wait_for_entry_observation(invalid_two, TcpIdleObservation::QueuedInput).await;
        {
            let mut inner = pool.inner.lock();
            // swap_remove visits invalid_one, invalid_two, then valid.
            inner.idle.push(invalid_one);
            inner.idle.push(valid);
            inner.idle.push(invalid_two);
            assert_eq!(inner.non_priority_total(), 3);
        }

        let (waiter, waiter_entered) =
            spawn_pending_timed_get(Arc::clone(&pool), waiting_addr, Duration::from_secs(5));
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        let reused = pool
            .get_with_connector(addr, || async { Err(TransportError::Disconnected) })
            .await
            .expect("checkout should skip adverse entries and reuse the clean one");
        waiter_entered
            .await
            .expect("passive retirements should wake the capacity waiter");
        assert_eq!(pool.active_count(), 2);
        assert_eq!(pool.idle_count(), 0);
        assert_eq!(pool.inner.lock().non_priority_total(), 2);

        abort_pending(waiter).await;
        assert_eq!(pool.active_count(), 1);
        drop(reused);
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
        assert_eq!(pool.inner.lock().non_priority_total(), 0);
    }

    #[tokio::test]
    async fn health_sweep_validates_both_pools_before_non_priority_age_eviction() {
        let _tracing_test = TRACING_TEST_LOCK.lock().await;
        let pool = ConnectionPool::new(test_config(8));
        let now = Instant::now();
        let expired = now - Duration::from_secs(2);

        let (priority_clean, _priority_clean_peer) =
            idle_entry_with_peer(test_addr(15_095), expired, true).await;
        let (priority_queued, mut priority_queued_peer) =
            idle_entry_with_peer(test_addr(15_096), now, true).await;
        let (non_priority_queued, mut non_priority_queued_peer) =
            idle_entry_with_peer(test_addr(15_097), now, false).await;
        let (non_priority_expired, _non_priority_expired_peer) =
            idle_entry_with_peer(test_addr(15_098), expired, false).await;
        let (non_priority_clean, _non_priority_clean_peer) =
            idle_entry_with_peer(test_addr(15_099), now, false).await;

        priority_queued_peer.write_all(&[0x01]).await.unwrap();
        non_priority_queued_peer.write_all(&[0x02]).await.unwrap();
        let priority_queued =
            wait_for_entry_observation(priority_queued, TcpIdleObservation::QueuedInput).await;
        let non_priority_queued =
            wait_for_entry_observation(non_priority_queued, TcpIdleObservation::QueuedInput).await;
        {
            let mut inner = pool.inner.lock();
            inner.idle = vec![
                priority_clean,
                priority_queued,
                non_priority_queued,
                non_priority_expired,
                non_priority_clean,
            ];
        }

        let capture = IdleRetirementCapture::default();
        let dispatch = tracing::Dispatch::new(registry().with(capture.clone()));
        let continued = dispatcher::with_default(&dispatch, || {
            health::run_health_check(
                &pool.inner,
                &pool.capacity_changed,
                now,
                Duration::from_secs(1),
            )
        });

        assert!(continued);
        let inner = pool.inner.lock();
        assert_eq!(inner.idle.len(), 2);
        assert!(
            inner
                .idle
                .iter()
                .any(|entry| entry.addr == test_addr(15_095) && entry.is_priority)
        );
        assert!(
            inner
                .idle
                .iter()
                .any(|entry| entry.addr == test_addr(15_099) && !entry.is_priority)
        );
        drop(inner);

        assert_eq!(
            capture.events.lock().unwrap().as_slice(),
            [
                CapturedIdleRetirement {
                    message: "idle_tcp_connection_passively_retired".to_owned(),
                    reason: "queued_input".to_owned(),
                    trigger: "health_sweep".to_owned(),
                    is_priority: true,
                },
                CapturedIdleRetirement {
                    message: "idle_tcp_connection_passively_retired".to_owned(),
                    reason: "queued_input".to_owned(),
                    trigger: "health_sweep".to_owned(),
                    is_priority: false,
                },
            ]
        );
    }

    #[tokio::test]
    async fn raw_drop_wakes_non_priority_waiter_for_fresh_connection() {
        let addr = test_addr(15_100);
        let pool = Arc::new(ConnectionPool::new(test_config(1)));
        let first = acquired_connection(&pool, addr).await;
        let replacement_halves = connected_halves().await;

        let waiting_pool = Arc::clone(&pool);
        let waiter = tokio::spawn(async move {
            waiting_pool
                .get_with_acquisition_timeout_and_connector(
                    addr,
                    Duration::from_secs(5),
                    move || async move { Ok(replacement_halves) },
                )
                .await
        });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        drop(first);

        let fresh = waiter
            .await
            .expect("waiter task should complete")
            .expect("raw retirement should release capacity for a fresh connection");
        assert_eq!(fresh.addr(), addr);
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.idle_count(), 0);

        drop(fresh);
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
    }

    #[tokio::test]
    async fn invalidation_wakes_waiter_and_runs_fresh_connector() {
        let addr = test_addr(15_101);
        let pool = Arc::new(ConnectionPool::new(test_config(1)));
        let mut first = acquired_connection(&pool, addr).await;
        let replacement_halves = connected_halves().await;
        let (entered_tx, mut entered_rx) = oneshot::channel();

        let waiting_pool = Arc::clone(&pool);
        let waiter = tokio::spawn(async move {
            waiting_pool
                .get_with_acquisition_timeout_and_connector(
                    addr,
                    Duration::from_secs(5),
                    move || async move {
                        entered_tx
                            .send(())
                            .expect("fresh connector entry receiver should remain live");
                        Ok(replacement_halves)
                    },
                )
                .await
        });
        tokio::task::yield_now().await;
        assert_eq!(entered_rx.try_recv(), Err(TryRecvError::Empty));
        assert!(!waiter.is_finished());

        first.invalidate(crate::LeaseInvalidationReason::Transport);
        entered_rx
            .await
            .expect("invalidation should wake the fresh connector");

        let replacement = waiter
            .await
            .expect("waiter task should complete")
            .expect("fresh connector should succeed");
        assert_eq!(replacement.addr(), addr);
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.idle_count(), 0);

        drop(first);
        drop(replacement);
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
    }

    #[tokio::test]
    async fn priority_waiter_ignores_cross_budget_wake_until_device_is_available() {
        let priority_addr = test_addr(15_102);
        let non_priority_addr = test_addr(15_103);
        let mut config = test_config(1);
        config.priority_devices = vec![crate::PriorityDevice {
            addr: priority_addr,
            max_connections: 1,
        }];
        let pool = Arc::new(ConnectionPool::new(config));
        let priority = acquired_connection(&pool, priority_addr).await;
        let non_priority = acquired_connection(&pool, non_priority_addr).await;
        let replacement_halves = connected_halves().await;

        let waiting_pool = Arc::clone(&pool);
        let waiter = tokio::spawn(async move {
            waiting_pool
                .get_with_acquisition_timeout_and_connector(
                    priority_addr,
                    Duration::from_secs(5),
                    move || async move { Ok(replacement_halves) },
                )
                .await
        });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        // A non-priority retirement broadcasts, but cannot satisfy this device's
        // separate priority budget.
        drop(non_priority);
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.idle_count(), 0);

        drop(priority);
        let acquired = waiter
            .await
            .expect("priority waiter task should complete")
            .expect("device return should satisfy its own waiter");
        assert_eq!(acquired.addr(), priority_addr);
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.idle_count(), 0);
    }

    #[tokio::test]
    async fn cancelled_pending_reservation_wakes_waiter_with_exact_accounting() {
        let pool = Arc::new(ConnectionPool::new(test_config(1)));
        let (first, first_entered) = spawn_pending_get(Arc::clone(&pool), test_addr(15_104));
        first_entered
            .await
            .expect("first connector should reserve the only slot");

        let (waiter, waiter_entered) =
            spawn_pending_timed_get(Arc::clone(&pool), test_addr(15_105), Duration::from_secs(5));
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        assert_eq!(pool.active_count(), 1);

        abort_pending(first).await;
        waiter_entered
            .await
            .expect("reservation cancellation should wake the waiter");
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.inner.lock().non_priority_total(), 1);

        abort_pending(waiter).await;
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.inner.lock().non_priority_total(), 0);
    }

    #[tokio::test]
    async fn explicit_connect_failure_wakes_waiter_with_exact_accounting() {
        let pool = Arc::new(ConnectionPool::new(test_config(1)));
        let (entered_tx, entered_rx) = oneshot::channel();
        let (fail_tx, fail_rx) = oneshot::channel();
        let first_pool = Arc::clone(&pool);
        let first = tokio::spawn(async move {
            first_pool
                .get_with_connector(test_addr(15_106), move || async move {
                    entered_tx
                        .send(())
                        .expect("first connector entry receiver should remain live");
                    fail_rx
                        .await
                        .expect("test should explicitly release connector failure");
                    Err(TransportError::Disconnected)
                })
                .await
        });
        entered_rx
            .await
            .expect("first connector should reserve the only slot");

        let (waiter, waiter_entered) =
            spawn_pending_timed_get(Arc::clone(&pool), test_addr(15_107), Duration::from_secs(5));
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        fail_tx
            .send(())
            .expect("first connector should still be pending");
        let result = first.await.expect("first connector task should complete");
        assert!(matches!(
            result,
            Err(PoolError::ConnectionFailed(TransportError::Disconnected))
        ));
        waiter_entered
            .await
            .expect("connect failure rollback should wake the waiter");
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.inner.lock().non_priority_total(), 1);

        abort_pending(waiter).await;
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.inner.lock().non_priority_total(), 0);
    }

    #[tokio::test]
    async fn health_eviction_broadcast_wakes_blocked_non_priority_waiter() {
        let idle_addr = test_addr(15_108);
        let replacement_addr = test_addr(15_109);
        let pool = Arc::new(ConnectionPool::new(test_config(1)));

        // A future timestamp keeps this synthetic idle entry from being selected
        // by the unchanged LRU rule. Running the same eviction pass used by the
        // health task at a later explicit instant makes the wake deterministic.
        let last_used = Instant::now() + Duration::from_mins(1);
        let entry = idle_entry(idle_addr, last_used).await;
        pool.inner.lock().idle.push(entry);
        let (waiter, mut waiter_entered) =
            spawn_pending_timed_get(Arc::clone(&pool), replacement_addr, Duration::from_secs(5));
        tokio::task::yield_now().await;
        assert_eq!(waiter_entered.try_recv(), Err(TryRecvError::Empty));
        assert!(!waiter.is_finished());

        assert!(health::run_health_check(
            &pool.inner,
            &pool.capacity_changed,
            last_used + Duration::from_secs(1),
            Duration::from_secs(1),
        ));
        waiter_entered
            .await
            .expect("actual health eviction should wake the waiter");
        assert_eq!(pool.idle_count(), 0);
        assert_eq!(pool.active_count(), 1);

        abort_pending(waiter).await;
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.inner.lock().non_priority_total(), 0);
    }

    #[tokio::test]
    async fn shutdown_broadcast_wakes_blocked_waiter() {
        let pool = Arc::new(ConnectionPool::new(test_config(1)));
        let first = acquired_connection(&pool, test_addr(15_110)).await;
        let (waiter, _waiter_entered) =
            spawn_pending_timed_get(Arc::clone(&pool), test_addr(15_111), Duration::from_secs(5));
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        pool.shutdown();
        let result = waiter.await.expect("waiter task should complete");
        assert!(matches!(result, Err(PoolError::ShuttingDown)));
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.idle_count(), 0);

        drop(first);
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
    }

    #[tokio::test]
    async fn spurious_broadcasts_do_not_extend_absolute_deadline() {
        let pool = Arc::new(ConnectionPool::new(test_config(0)));
        let waiting_pool = Arc::clone(&pool);
        let waiter = tokio::spawn(async move {
            waiting_pool
                .get_with_acquisition_timeout_and_connector(
                    test_addr(15_112),
                    Duration::from_millis(100),
                    || async { Err(TransportError::Disconnected) },
                )
                .await
        });
        tokio::task::yield_now().await;

        let broadcasting_pool = Arc::clone(&pool);
        let broadcaster = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(10)).await;
                broadcasting_pool.capacity_changed.notify_waiters();
            }
        });

        let result = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("absolute acquisition deadline should remain bounded")
            .expect("waiter task should complete");
        assert!(matches!(result, Err(PoolError::Timeout)));
        broadcaster.abort();
        let error = broadcaster
            .await
            .expect_err("broadcaster should be cancelled after the waiter finishes");
        assert!(error.is_cancelled());
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
    }

    #[tokio::test]
    async fn cancelling_capacity_wait_changes_no_accounting_and_strands_no_later_waiter() {
        let pool = Arc::new(ConnectionPool::new(test_config(1)));
        let (first, first_entered) = spawn_pending_get(Arc::clone(&pool), test_addr(15_113));
        first_entered
            .await
            .expect("first connector should reserve the only slot");

        let (cancelled_waiter, _cancelled_entered) =
            spawn_pending_timed_get(Arc::clone(&pool), test_addr(15_114), Duration::from_secs(5));
        tokio::task::yield_now().await;
        assert!(!cancelled_waiter.is_finished());
        abort_pending(cancelled_waiter).await;
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.inner.lock().non_priority_total(), 1);

        let (later_waiter, later_entered) =
            spawn_pending_timed_get(Arc::clone(&pool), test_addr(15_115), Duration::from_secs(5));
        tokio::task::yield_now().await;
        assert!(!later_waiter.is_finished());

        abort_pending(first).await;
        later_entered
            .await
            .expect("later waiter should observe released capacity");
        assert_eq!(pool.active_count(), 1);

        abort_pending(later_waiter).await;
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.inner.lock().non_priority_total(), 0);
    }

    #[tokio::test]
    async fn acquisition_deadline_ends_before_pending_connector() {
        let pool = Arc::new(ConnectionPool::new(test_config(1)));
        let (task, entered) = spawn_pending_timed_get(
            Arc::clone(&pool),
            test_addr(15_116),
            Duration::from_millis(20),
        );
        entered
            .await
            .expect("immediate reservation should enter the connector");
        assert_eq!(pool.active_count(), 1);

        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(
            !task.is_finished(),
            "capacity timeout must not wrap the pending connector"
        );
        assert_eq!(pool.active_count(), 1);

        abort_pending(task).await;
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.inner.lock().non_priority_total(), 0);
    }

    #[tokio::test]
    async fn connector_timeout_after_reservation_remains_connection_failed() {
        let pool = ConnectionPool::new(test_config(1));

        let result = pool
            .get_with_acquisition_timeout_and_connector(
                test_addr(15_118),
                Duration::ZERO,
                || async { Err(TransportError::Timeout) },
            )
            .await;

        assert!(matches!(
            result,
            Err(PoolError::ConnectionFailed(TransportError::Timeout))
        ));
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.inner.lock().non_priority_total(), 0);
    }

    #[tokio::test]
    async fn multiple_waiters_and_raw_retirements_do_not_strand_available_capacity() {
        const WAITERS: usize = 8;

        let addr = test_addr(15_117);
        let pool = Arc::new(ConnectionPool::new(test_config(2)));
        let first = acquired_connection(&pool, addr).await;
        let second = acquired_connection(&pool, addr).await;
        let mut waiters = Vec::new();

        for _ in 0..WAITERS {
            let waiting_pool = Arc::clone(&pool);
            waiters.push(tokio::spawn(async move {
                let connection = waiting_pool
                    .get_with_acquisition_timeout_and_connector(
                        addr,
                        Duration::from_secs(5),
                        || async { Ok(connected_halves().await) },
                    )
                    .await?;
                tokio::task::yield_now().await;
                drop(connection);
                Ok::<(), PoolError>(())
            }));
        }
        tokio::task::yield_now().await;
        assert!(waiters.iter().all(|waiter| !waiter.is_finished()));

        drop(first);
        drop(second);

        for waiter in waiters {
            waiter
                .await
                .expect("waiter task should complete")
                .expect("each waiter should eventually reserve released capacity");
        }
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
        assert_eq!(pool.inner.lock().non_priority_total(), 0);
    }

    #[tokio::test]
    async fn cancel_pending_non_priority_connect_releases_unused_capacity() {
        let pool = Arc::new(ConnectionPool::new(test_config(1)));
        let (task, entered) = spawn_pending_get(Arc::clone(&pool), test_addr(15_001));

        entered
            .await
            .expect("connector should be polled after reserving capacity");
        {
            let inner = pool.inner.lock();
            assert_eq!(inner.active_non_priority, 1);
            assert_eq!(inner.non_priority_total(), 1);
        }

        abort_pending(task).await;

        let inner = pool.inner.lock();
        assert_eq!(inner.active_non_priority, 0);
        assert_eq!(inner.non_priority_total(), 0);
        assert_eq!(inner.active_total(), 0);
    }

    #[tokio::test]
    async fn cancel_pending_priority_connect_releases_device_budget() {
        let addr = test_addr(15_002);
        let mut config = test_config(1);
        config.priority_devices = vec![crate::PriorityDevice {
            addr,
            max_connections: 1,
        }];
        let pool = Arc::new(ConnectionPool::new(config));

        let (first_task, first_entered) = spawn_pending_get(Arc::clone(&pool), addr);
        first_entered
            .await
            .expect("first connector should enter after reserving priority capacity");
        assert_eq!(pool.inner.lock().active_priority.get(&addr), Some(&1));
        abort_pending(first_task).await;
        assert!(!pool.inner.lock().active_priority.contains_key(&addr));

        // Reaching a second pending connector proves the one-slot device budget
        // was released rather than failing fast with Exhausted.
        let (second_task, second_entered) = spawn_pending_get(Arc::clone(&pool), addr);
        second_entered
            .await
            .expect("later reservation should reach its connector");
        assert_eq!(pool.inner.lock().active_priority.get(&addr), Some(&1));
        abort_pending(second_task).await;

        let inner = pool.inner.lock();
        assert!(!inner.active_priority.contains_key(&addr));
        assert_eq!(inner.active_total(), 0);
    }

    #[tokio::test]
    async fn cancel_after_lru_eviction_restores_original_entry() {
        let oldest_addr = test_addr(15_003);
        let newer_addr = test_addr(15_004);
        let replacement_addr = test_addr(15_005);
        let pool = Arc::new(ConnectionPool::new(test_config(2)));

        let oldest_last_used = Instant::now();
        tokio::time::sleep(Duration::from_millis(2)).await;
        let newer_last_used = Instant::now();
        let oldest = idle_entry(oldest_addr, oldest_last_used).await;
        let newer = idle_entry(newer_addr, newer_last_used).await;
        {
            let mut inner = pool.inner.lock();
            inner.idle.push(oldest);
            inner.idle.push(newer);
            assert_eq!(inner.non_priority_total(), 2);
        }

        let (task, entered) = spawn_pending_get(Arc::clone(&pool), replacement_addr);
        entered
            .await
            .expect("connector should be polled after evicting the LRU entry");
        {
            let inner = pool.inner.lock();
            assert_eq!(inner.active_non_priority, 1);
            assert_eq!(inner.idle.len(), 1);
            assert_eq!(inner.idle[0].addr, newer_addr);
            assert_eq!(inner.non_priority_total(), 2);
        }

        abort_pending(task).await;

        let inner = pool.inner.lock();
        assert_eq!(inner.active_non_priority, 0);
        assert_eq!(inner.non_priority_total(), 2);
        assert_eq!(inner.idle.len(), 2);
        let restored = inner
            .idle
            .iter()
            .find(|entry| entry.addr == oldest_addr)
            .expect("oldest evicted entry should be restored");
        assert!(!restored.is_priority);
        assert_eq!(restored.last_used, oldest_last_used);
    }

    #[tokio::test]
    async fn shutdown_then_cancel_releases_without_restoring_eviction() {
        let idle_addr = test_addr(15_006);
        let replacement_addr = test_addr(15_007);
        let pool = Arc::new(ConnectionPool::new(test_config(1)));
        let entry = idle_entry(idle_addr, Instant::now()).await;
        pool.inner.lock().idle.push(entry);

        let (task, entered) = spawn_pending_get(Arc::clone(&pool), replacement_addr);
        entered
            .await
            .expect("connector should be pending with the idle entry evicted");
        assert_eq!(pool.inner.lock().active_non_priority, 1);

        pool.shutdown();
        abort_pending(task).await;

        let inner = pool.inner.lock();
        assert!(inner.shutting_down);
        assert_eq!(inner.active_non_priority, 0);
        assert_eq!(inner.active_total(), 0);
        assert!(inner.idle.is_empty());
        assert_eq!(inner.non_priority_total(), 0);
    }

    #[tokio::test]
    async fn explicit_connect_failure_rolls_back_through_reservation() {
        let idle_addr = test_addr(15_008);
        let replacement_addr = test_addr(15_009);
        let pool = ConnectionPool::new(test_config(1));
        let original_last_used = Instant::now();
        let entry = idle_entry(idle_addr, original_last_used).await;
        pool.inner.lock().idle.push(entry);

        let result = pool
            .get_with_connector(replacement_addr, || async {
                Err(TransportError::Disconnected)
            })
            .await;
        assert!(matches!(
            result,
            Err(PoolError::ConnectionFailed(TransportError::Disconnected))
        ));

        let inner = pool.inner.lock();
        assert_eq!(inner.active_non_priority, 0);
        assert_eq!(inner.non_priority_total(), 1);
        assert_eq!(inner.idle.len(), 1);
        assert_eq!(inner.idle[0].addr, idle_addr);
        assert!(!inner.idle[0].is_priority);
        assert_eq!(inner.idle[0].last_used, original_last_used);
    }

    #[tokio::test]
    async fn successful_connect_commits_without_restoring_eviction() {
        let idle_addr = test_addr(15_010);
        let replacement_addr = test_addr(15_011);
        let pool = ConnectionPool::new(test_config(1));
        let entry = idle_entry(idle_addr, Instant::now()).await;
        pool.inner.lock().idle.push(entry);
        let replacement_halves = connected_halves().await;

        let connection = pool
            .get_with_connector(
                replacement_addr,
                move || async move { Ok(replacement_halves) },
            )
            .await
            .unwrap();
        assert_eq!(connection.addr(), replacement_addr);
        {
            let inner = pool.inner.lock();
            assert_eq!(inner.active_non_priority, 1);
            assert!(inner.idle.is_empty());
            assert_eq!(inner.non_priority_total(), 1);
        }

        drop(connection);

        let inner = pool.inner.lock();
        assert_eq!(inner.active_non_priority, 0);
        assert!(inner.idle.is_empty());
        assert_eq!(inner.non_priority_total(), 0);
    }

    #[tokio::test]
    async fn repeated_eviction_cancellation_stays_within_capacity() {
        const ATTEMPTS: usize = 16;

        let idle_addr = test_addr(15_012);
        let replacement_addr = test_addr(15_013);
        let pool = Arc::new(ConnectionPool::new(test_config(1)));
        let original_last_used = Instant::now();
        let entry = idle_entry(idle_addr, original_last_used).await;
        pool.inner.lock().idle.push(entry);

        for _ in 0..ATTEMPTS {
            let (task, entered) = spawn_pending_get(Arc::clone(&pool), replacement_addr);
            entered
                .await
                .expect("every attempt should reserve and enter its connector");
            {
                let inner = pool.inner.lock();
                assert_eq!(inner.active_non_priority, 1);
                assert!(inner.idle.is_empty());
                assert_eq!(inner.non_priority_total(), 1);
                assert!(inner.non_priority_total() <= pool.config.max_connections);
            }

            abort_pending(task).await;

            let inner = pool.inner.lock();
            assert_eq!(inner.active_non_priority, 0);
            assert_eq!(inner.active_total(), 0);
            assert_eq!(inner.non_priority_total(), 1);
            assert_eq!(inner.idle.len(), 1);
            assert_eq!(inner.idle[0].addr, idle_addr);
            assert_eq!(inner.idle[0].last_used, original_last_used);
            assert!(inner.non_priority_total() <= pool.config.max_connections);
        }
    }
}
