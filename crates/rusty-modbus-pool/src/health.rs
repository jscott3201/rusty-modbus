//! Background health check task for idle connections.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::Notify;
use tokio::time::Instant;

use crate::pool::PoolInner;

/// Spawn a background task that periodically evicts idle connections
/// that have exceeded the idle timeout.
pub(crate) fn spawn_health_check(
    inner: Arc<Mutex<PoolInner>>,
    capacity_changed: Arc<Notify>,
    interval: Duration,
    idle_timeout: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.tick().await; // First tick is immediate — skip it.

        loop {
            ticker.tick().await;

            let now = Instant::now();
            if !run_health_check(&inner, &capacity_changed, now, idle_timeout) {
                break;
            }
        }
    })
}

/// Run one idle-eviction pass. Returns `false` once shutdown is sticky.
pub(crate) fn run_health_check(
    inner: &Mutex<PoolInner>,
    capacity_changed: &Notify,
    now: Instant,
    idle_timeout: Duration,
) -> bool {
    let mut pool = inner.lock();
    if pool.shutting_down {
        return false;
    }

    // Remove non-priority connections that have been idle too long.
    let idle_before = pool.idle.len();
    pool.idle.retain(|entry| {
        if entry.is_priority {
            return true;
        }
        now.duration_since(entry.last_used) < idle_timeout
    });
    let capacity_freed = pool.idle.len() < idle_before;
    drop(pool);
    if capacity_freed {
        capacity_changed.notify_waiters();
    }
    true
}
