//! Background health check task for idle connections.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::Notify;
use tokio::time::Instant;

use crate::pool::{
    IdleEntryInspection, IdleValidationTrigger, PoolInner, finish_passive_retirements,
    inspect_idle_entry,
};

/// Spawn a background task that periodically validates idle connections and
/// age-evicts expired non-priority entries.
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

/// Run one passive idle-validation and age-eviction pass.
///
/// This synchronous pass never awaits socket readiness. It retires any idle
/// priority or non-priority entry with an immediately observable adverse signal,
/// retains no-adverse-signal priority entries regardless of age, and age-evicts
/// expired no-adverse-signal non-priority entries. Returns `false` once shutdown
/// is sticky.
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

    let idle = std::mem::take(&mut pool.idle);
    let mut retained = Vec::with_capacity(idle.len());
    let mut age_evictions = Vec::new();
    let mut passive_retirements = Vec::new();

    for entry in idle {
        match inspect_idle_entry(entry) {
            IdleEntryInspection::Retain(entry)
                if entry.is_priority || now.duration_since(entry.last_used) < idle_timeout =>
            {
                retained.push(entry);
            }
            IdleEntryInspection::Retain(entry) => age_evictions.push(entry),
            IdleEntryInspection::Retire(entry, observation) => {
                passive_retirements.push((entry, observation));
            }
        }
    }
    pool.idle = retained;
    let retirement_count = age_evictions
        .len()
        .saturating_add(passive_retirements.len());
    pool.record_connections_retired(retirement_count);
    drop(pool);

    let passively_freed = !passive_retirements.is_empty();
    let age_freed = !age_evictions.is_empty();
    drop(age_evictions);
    finish_passive_retirements(
        passive_retirements,
        IdleValidationTrigger::HealthSweep,
        capacity_changed,
    );
    if !passively_freed && age_freed {
        capacity_changed.notify_waiters();
    }
    true
}
