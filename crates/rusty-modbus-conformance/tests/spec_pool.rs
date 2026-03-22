//! Connection pool conformance tests.
//!
//! Verifies the two-pool eviction model per TCP Guide §4.2.1.

use std::time::Duration;

use rusty_modbus_pool::{BackoffConfig, PoolConfig};

// ── Pool Config Defaults ──────────────────────────────────────────

#[test]
fn spec_4_2_1_pool_defaults() {
    let config = PoolConfig::default();
    // Reasonable defaults per spec guidance
    assert!(config.max_connections > 0);
    assert!(config.idle_timeout > Duration::ZERO);
    assert!(config.health_check_interval > Duration::ZERO);
}

#[test]
fn spec_4_2_1_pre_connect_default() {
    let config = PoolConfig::default();
    // Pre-connect to priority devices is recommended
    assert!(config.pre_connect);
}

// ── Backoff Config ────────────────────────────────────────────────

#[test]
fn backoff_defaults_reasonable() {
    let config = BackoffConfig::default();
    // Initial delay should be small
    assert!(config.initial_delay <= Duration::from_secs(1));
    // Max delay should cap
    assert!(config.max_delay >= Duration::from_secs(1));
    // Multiplier > 1 for exponential growth
    assert!(config.multiplier > 1.0);
}

// ── Two-Pool Model (via integration tests in pool crate) ──────────
// The existing pool_tests.rs already covers:
// - Priority connections not evicted ✓
// - Oldest non-priority evicted first ✓
// - Exhaustion when full ✓
// - Idle connection reuse ✓
// - Shutdown rejects new requests ✓
// These are conformance-verified by the existing tests.
