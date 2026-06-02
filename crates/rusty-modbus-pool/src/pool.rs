//! Connection pool — two-pool model per TCP Guide §4.2.1.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::time::Instant;

use rusty_modbus_tcp::{TcpRecvStream, TcpSink, TcpTransport, TransportConnect};

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

/// Shared mutable pool state.
///
/// The two pools are accounted **separately** (TCP Guide §4.2.1): non-priority
/// connections are bounded by [`PoolConfig::max_connections`], while each
/// priority device has its own budget ([`PriorityDevice::max_connections`](crate::PriorityDevice::max_connections)).
/// Keeping them separate is what stops never-evicted idle priority connections
/// from starving non-priority requests.
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

/// Connection pool with two-pool eviction model.
///
/// Priority connections (to configured addresses) are never locally evicted and
/// live in a per-device budget separate from the non-priority pool. Non-priority
/// connections are evicted oldest-first when the non-priority pool is full.
pub struct ConnectionPool {
    inner: Arc<Mutex<PoolInner>>,
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

        let health_task = health::spawn_health_check(
            Arc::clone(&inner),
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
            config,
            health_task: Some(health_task),
            pre_connect_tasks,
        }
    }

    /// Get a connection to the specified address.
    ///
    /// Returns an idle connection if available, otherwise establishes a new one.
    /// Priority devices draw from their own per-device budget; non-priority
    /// requests draw from `max_connections`, evicting the oldest idle
    /// non-priority connection when that pool is full.
    ///
    /// # Errors
    ///
    /// - [`PoolError::ShuttingDown`] if the pool is shutting down.
    /// - [`PoolError::Exhausted`] if the relevant pool is full and no connection
    ///   can be evicted (priority devices are never evicted, so a priority
    ///   request fails once the device hits its own `max_connections`).
    /// - [`PoolError::ConnectionFailed`] if establishing a new connection fails.
    pub async fn get(&self, addr: SocketAddr) -> Result<PooledConnection, PoolError> {
        let is_priority = self.is_priority_addr(addr);
        // An idle non-priority connection evicted to make room. Held (not
        // dropped) across the connect so a transient connect failure can restore
        // it instead of destroying a healthy pooled connection.
        let mut evicted: Option<PoolEntry> = None;

        // Fast path: reuse an idle connection or reserve a slot under the lock.
        {
            let mut inner = self.inner.lock();
            if inner.shutting_down {
                return Err(PoolError::ShuttingDown);
            }

            // Reuse an idle connection to this address (either pool).
            if let Some(idx) = inner.idle.iter().position(|e| e.addr == addr) {
                let mut entry = inner.idle.swap_remove(idx);
                entry.last_used = Instant::now();
                inner.charge_active(&entry);
                return Ok(PooledConnection::new(entry, Arc::clone(&self.inner)));
            }

            // No idle connection: reserve a slot in the appropriate pool.
            if is_priority {
                // Priority pool: bounded by this device's own budget, never evicted.
                let cap = self.priority_cap(addr);
                if inner.priority_total(addr) >= cap {
                    return Err(PoolError::Exhausted);
                }
                *inner.active_priority.entry(addr).or_insert(0) += 1;
            } else {
                // Non-priority pool: bounded by max_connections, LRU-evicted.
                if inner.non_priority_total() < self.config.max_connections {
                    inner.active_non_priority += 1;
                } else if let Some(idx) = find_evictable(&inner) {
                    // Evict to stay within the cap, but keep the entry in hand:
                    // if the new connect fails we put it back.
                    evicted = Some(inner.idle.swap_remove(idx));
                    inner.active_non_priority += 1;
                } else {
                    return Err(PoolError::Exhausted);
                }
            }
        }
        // Lock released — connect outside the lock.

        match TcpTransport::connect(self.config.tcp_config.clone(), addr).await {
            Ok((sink, stream)) => {
                // Connect succeeded: the evicted connection (if any) is dropped
                // here, closing its socket and keeping the cap satisfied.
                let entry = PoolEntry {
                    addr,
                    sink,
                    stream,
                    last_used: Instant::now(),
                    is_priority,
                };
                Ok(PooledConnection::new(entry, Arc::clone(&self.inner)))
            }
            Err(e) => {
                let mut inner = self.inner.lock();
                inner.release_active(is_priority, addr);
                // Restore the healthy connection we evicted — the failure was the
                // new connect, not the pooled one.
                if let Some(entry) = evicted
                    && !inner.shutting_down
                {
                    inner.idle.push(entry);
                }
                Err(PoolError::ConnectionFailed(e))
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
