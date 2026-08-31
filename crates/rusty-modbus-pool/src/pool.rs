//! Connection pool — two-pool model per TCP Guide §4.2.1.

use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::Notify;
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
    /// Number of currently checked-out (active) **non-priority** connections.
    pub active_non_priority: usize,
    /// Number of currently checked-out (active) **priority** connections,
    /// counted per device address so each device's budget is independent.
    pub active_priority: HashMap<SocketAddr, usize>,
    /// Whether the pool is shutting down.
    pub shutting_down: bool,
}

impl PoolInner {
    /// Total active (checked-out) connections across both pools.
    pub(crate) fn active_total(&self) -> usize {
        self.active_non_priority + self.active_priority.values().sum::<usize>()
    }

    /// Active + idle connections to a specific priority device (its pool size).
    pub(crate) fn priority_total(&self, addr: SocketAddr) -> usize {
        let active = self.active_priority.get(&addr).copied().unwrap_or(0);
        let idle = self
            .idle
            .iter()
            .filter(|e| e.is_priority && e.addr == addr)
            .count();
        active + idle
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

    /// Decrement the active counter when a connection is returned or a pending
    /// connect attempt fails.
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
/// Unless committed into a [`PooledConnection`], dropping the reservation
/// releases the charge and restores any idle entry tentatively evicted to make
/// room. Rollback is synchronous so dropping a pending `get` future is enough
/// to restore the pool invariants.
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
    health_task: Option<tokio::task::JoinHandle<()>>,
    /// Background pre-connect/reconnect tasks (one per priority device).
    /// Tracked so they can be aborted on shutdown — otherwise a task parked in
    /// its backoff sleep would outlive the pool and keep `inner` alive.
    pre_connect_tasks: Vec<tokio::task::JoinHandle<()>>,
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
    /// If `config.pre_connect` is true and priority devices are configured,
    /// background tasks are spawned to eagerly establish those connections,
    /// retrying with exponential backoff until they succeed or the pool shuts
    /// down. The call itself returns immediately.
    #[must_use]
    pub fn new(config: PoolConfig) -> Self {
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

        let pre_connect_tasks = if config.pre_connect {
            Self::spawn_pre_connect(&config, &inner)
        } else {
            Vec::new()
        };

        Self {
            inner,
            capacity_changed,
            config,
            health_task: Some(health_task),
            pre_connect_tasks,
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

    /// Shut down the pool — drop all idle connections and reject future requests.
    ///
    /// Aborts the background health-check and all pre-connect/reconnect tasks so
    /// they stop immediately (even if parked in a backoff sleep) and release
    /// their references to the pool state.
    pub fn shutdown(&self) {
        {
            let mut inner = self.inner.lock();
            inner.shutting_down = true;
            inner.idle.clear();
        }
        self.capacity_changed.notify_waiters();

        if let Some(task) = &self.health_task {
            task.abort();
        }
        for task in &self.pre_connect_tasks {
            task.abort();
        }
    }

    /// Number of currently active (checked-out) connections across both pools.
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

    /// Spawn background tasks to pre-connect to priority devices.
    ///
    /// One task per *distinct* device address (duplicate `priority_devices`
    /// entries for the same address are ignored so they cannot collectively
    /// exceed the per-device budget). Each task retries with exponential
    /// [`Backoff`] until it establishes a connection (capped at the device's
    /// per-device budget) or the pool shuts down. This is where
    /// [`BackoffConfig`](crate::BackoffConfig) is applied. The returned handles
    /// are aborted by [`shutdown`](Self::shutdown).
    fn spawn_pre_connect(
        config: &PoolConfig,
        inner: &Arc<Mutex<PoolInner>>,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        let mut seen = std::collections::HashSet::new();
        let mut handles = Vec::new();

        for pd in &config.priority_devices {
            if !seen.insert(pd.addr) {
                continue; // already spawning for this address
            }
            let addr = pd.addr;
            let cap = pd.max_connections;
            let tcp_config = config.tcp_config.clone();
            let backoff_config = config.backoff.clone();
            let inner = Arc::clone(inner);

            handles.push(tokio::spawn(async move {
                let mut backoff = Backoff::new(backoff_config);
                loop {
                    // Stop retrying once the pool is shutting down.
                    if inner.lock().shutting_down {
                        return;
                    }

                    if let Ok((sink, stream)) =
                        TcpTransport::connect(tcp_config.clone(), addr).await
                    {
                        let entry = PoolEntry {
                            addr,
                            sink,
                            stream,
                            last_used: Instant::now(),
                            is_priority: true,
                        };
                        let mut pool = inner.lock();
                        // Respect shutdown and the device's own budget.
                        if !pool.shutting_down && pool.priority_total(addr) < cap {
                            pool.idle.push(entry);
                        }
                        return;
                    }

                    // Connect failed: reconnect with exponential backoff (capped
                    // at max_delay), then loop to re-check shutdown and retry. A
                    // minimum 1ms floor guards against a zero `initial_delay`
                    // config turning this into a hot busy-loop.
                    let delay = backoff.next_delay().max(MIN_BACKOFF_SLEEP);
                    tokio::time::sleep(delay).await;
                }
            }));
        }

        handles
    }
}

/// Lower bound on the pre-connect reconnect sleep, preventing a misconfigured
/// zero `initial_delay` from spinning the retry loop.
const MIN_BACKOFF_SLEEP: std::time::Duration = std::time::Duration::from_millis(1);

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
    use std::future::pending;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    use rusty_modbus_frame::FrameHeader;
    use rusty_modbus_tcp::TransportStream;
    use rusty_modbus_tcp::{TcpConfig, TransportError};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::oneshot;
    use tokio::sync::oneshot::error::TryRecvError;
    use tokio::task::JoinHandle;
    use tracing::field::{Field, Visit};
    use tracing::{Event, Subscriber, dispatcher};
    use tracing_subscriber::layer::Context;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{Layer, registry};

    type GetTask = JoinHandle<Result<PooledConnection, PoolError>>;

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
        assert_eq!(pool.idle_count(), 1);
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
        assert_eq!(pool.idle_count(), 1);
        assert_eq!(pool.inner.lock().non_priority_total(), 1);
    }

    #[tokio::test]
    async fn health_sweep_validates_both_pools_before_non_priority_age_eviction() {
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
    async fn healthy_return_wakes_non_priority_waiter_for_reuse() {
        let addr = test_addr(15_100);
        let pool = Arc::new(ConnectionPool::new(test_config(1)));
        let first = acquired_connection(&pool, addr).await;

        let waiting_pool = Arc::clone(&pool);
        let waiter = tokio::spawn(async move {
            waiting_pool
                .get_with_acquisition_timeout_and_connector(
                    addr,
                    Duration::from_secs(5),
                    || async { Err(TransportError::Disconnected) },
                )
                .await
        });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        drop(first);

        let reused = waiter
            .await
            .expect("waiter task should complete")
            .expect("returned lease should be reused without running the connector");
        assert_eq!(reused.addr(), addr);
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.idle_count(), 0);

        drop(reused);
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 1);
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
        assert_eq!(pool.idle_count(), 1);
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

        let waiting_pool = Arc::clone(&pool);
        let waiter = tokio::spawn(async move {
            waiting_pool
                .get_with_acquisition_timeout_and_connector(
                    priority_addr,
                    Duration::from_secs(5),
                    || async { Err(TransportError::Disconnected) },
                )
                .await
        });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        // A non-priority return broadcasts, but cannot satisfy this device's
        // separate priority budget.
        drop(non_priority);
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.idle_count(), 1);

        drop(priority);
        let acquired = waiter
            .await
            .expect("priority waiter task should complete")
            .expect("device return should satisfy its own waiter");
        assert_eq!(acquired.addr(), priority_addr);
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.idle_count(), 1);
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
    async fn multiple_waiters_and_returns_do_not_strand_available_capacity() {
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
                        || async { Err(TransportError::Disconnected) },
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
                .expect("each waiter should eventually reuse released capacity");
        }
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 2);
        assert_eq!(pool.inner.lock().non_priority_total(), 2);
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
        assert_eq!(inner.idle.len(), 1);
        assert_eq!(inner.idle[0].addr, replacement_addr);
        assert_eq!(inner.non_priority_total(), 1);
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
