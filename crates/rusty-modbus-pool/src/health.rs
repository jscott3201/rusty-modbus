//! Background health check task for idle connections.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::time::Instant;

use crate::pool::PoolInner;

/// Spawn a background task that periodically evicts idle connections
/// that have exceeded the idle timeout.
pub(crate) fn spawn_health_check(
    inner: Arc<Mutex<PoolInner>>,
    interval: Duration,
    idle_timeout: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.tick().await; // First tick is immediate — skip it.

        loop {
            ticker.tick().await;

            let now = Instant::now();
            let mut pool = inner.lock();
            if pool.shutting_down {
                break;
            }

            // Remove non-priority connections that have been idle too long.
            pool.idle.retain(|entry| {
                if entry.is_priority {
                    return true;
                }
                now.duration_since(entry.last_used) < idle_timeout
            });
        }
    })
}
