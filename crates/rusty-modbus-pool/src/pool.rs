//! Connection pool — two-pool model per TCP Guide §4.2.1.

use std::net::SocketAddr;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::time::Instant;

use rusty_modbus_tcp::{TcpSink, TcpRecvStream, TcpTransport, TransportConnect};

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
pub(crate) struct PoolInner {
    /// Idle connections available for reuse.
    pub idle: Vec<PoolEntry>,
    /// Number of currently checked-out (active) connections.
    pub active_count: usize,
    /// Whether the pool is shutting down.
    pub shutting_down: bool,
}

/// Connection pool with two-pool eviction model.
///
/// Priority connections (to configured addresses) are never locally evicted.
/// Non-priority connections are evicted oldest-first when the pool is full.
pub struct ConnectionPool {
    inner: Arc<Mutex<PoolInner>>,
    config: PoolConfig,
    health_task: Option<tokio::task::JoinHandle<()>>,
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
    /// background tasks are spawned to eagerly establish those connections.
    /// The call itself returns immediately.
    #[must_use]
    pub fn new(config: PoolConfig) -> Self {
        let inner = Arc::new(Mutex::new(PoolInner {
            idle: Vec::new(),
            active_count: 0,
            shutting_down: false,
        }));

        let health_task = health::spawn_health_check(
            Arc::clone(&inner),
            config.health_check_interval,
            config.idle_timeout,
        );

        let pool = Self {
            inner,
            config,
            health_task: Some(health_task),
        };

        if pool.config.pre_connect {
            pool.spawn_pre_connect();
        }

        pool
    }

    /// Get a connection to the specified address.
    ///
    /// Returns an idle connection if available, otherwise establishes a new one.
    /// If the pool is full, evicts the oldest idle non-priority connection.
    ///
    /// # Errors
    ///
    /// - [`PoolError::ShuttingDown`] if the pool is shutting down.
    /// - [`PoolError::Exhausted`] if the pool is full and no connections can be evicted.
    /// - [`PoolError::ConnectionFailed`] if establishing a new connection fails.
    pub async fn get(&self, addr: SocketAddr) -> Result<PooledConnection, PoolError> {
        // Fast path: check for idle connection or capacity.
        {
            let mut inner = self.inner.lock();
            if inner.shutting_down {
                return Err(PoolError::ShuttingDown);
            }

            // Look for an idle connection to this address.
            if let Some(idx) = inner.idle.iter().position(|e| e.addr == addr) {
                let mut entry = inner.idle.swap_remove(idx);
                entry.last_used = Instant::now();
                inner.active_count += 1;
                return Ok(PooledConnection::new(entry, Arc::clone(&self.inner)));
            }

            let total = inner.active_count + inner.idle.len();
            if total < self.config.max_connections {
                inner.active_count += 1;
            } else if let Some(idx) = find_evictable(&inner) {
                inner.idle.swap_remove(idx);
                inner.active_count += 1;
            } else {
                return Err(PoolError::Exhausted);
            }
        }
        // Lock released — connect outside the lock.

        match TcpTransport::connect(self.config.tcp_config.clone(), addr).await {
            Ok((sink, stream)) => {
                let is_priority = self.is_priority_addr(addr);
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
                inner.active_count = inner.active_count.saturating_sub(1);
                Err(PoolError::ConnectionFailed(e))
            }
        }
    }

    /// Shut down the pool — drop all idle connections and reject future requests.
    pub fn shutdown(&self) {
        let mut inner = self.inner.lock();
        inner.shutting_down = true;
        inner.idle.clear();

        if let Some(task) = &self.health_task {
            task.abort();
        }
    }

    /// Number of currently active (checked-out) connections.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.inner.lock().active_count
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

    /// Spawn background tasks to pre-connect to priority devices.
    fn spawn_pre_connect(&self) {
        for pd in &self.config.priority_devices {
            let addr = pd.addr;
            let tcp_config = self.config.tcp_config.clone();
            let inner = Arc::clone(&self.inner);

            tokio::spawn(async move {
                if let Ok((sink, stream)) = TcpTransport::connect(tcp_config, addr).await {
                    let entry = PoolEntry {
                        addr,
                        sink,
                        stream,
                        last_used: Instant::now(),
                        is_priority: true,
                    };
                    let mut pool = inner.lock();
                    if !pool.shutting_down {
                        pool.idle.push(entry);
                    }
                }
                // Pre-connect failure is non-fatal; get() will retry.
            });
        }
    }
}

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
