//! Transaction manager for Modbus/TCP pipelining.
//!
//! Manages up to 16 concurrent in-flight transactions, matched by Transaction ID.
//! Uses a fixed-size ring indexed by `transaction_id % MAX_SLOTS` to avoid `HashMap` overhead.

use std::sync::atomic::{AtomicU16, Ordering};

use parking_lot::Mutex;
use rusty_modbus_frame::OwnedResponsePdu;
use rusty_modbus_types::{FunctionCode, TransactionId};
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::error::ClientError;

/// Maximum number of concurrent in-flight transactions.
pub(crate) const MAX_SLOTS: usize = 16;

/// A pending transaction waiting for a response.
pub(crate) struct PendingTransaction {
    /// The transaction ID this slot was registered for.
    pub txn_id: TransactionId,
    /// Channel to send the response (or error) to the caller.
    pub sender: oneshot::Sender<Result<OwnedResponsePdu, ClientError>>,
    /// When the request was sent.
    pub sent_at: Instant,
    /// The function code of the request (used by metrics feature).
    #[allow(dead_code)]
    pub function_code: FunctionCode,
}

/// Fixed-size transaction manager for pipelining.
pub(crate) struct TransactionManager {
    next_id: AtomicU16,
    slots: [Mutex<Option<PendingTransaction>>; MAX_SLOTS],
}

impl TransactionManager {
    /// Create a new transaction manager.
    pub fn new() -> Self {
        Self {
            next_id: AtomicU16::new(1), // Keep 0 reserved; some devices treat it specially.
            slots: std::array::from_fn(|_| Mutex::new(None)),
        }
    }

    fn next_transaction_id(&self) -> TransactionId {
        loop {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            if id != 0 {
                return TransactionId(id);
            }
        }
    }

    /// Allocate a new transaction ID and register a pending transaction.
    ///
    /// Tries sequential IDs until a free slot is found, avoiding
    /// `TransactionConflict` when slow responses occupy a slot whose
    /// modular index would collide with the next sequential ID.
    ///
    /// Returns the transaction ID and a receiver that will yield the response.
    ///
    /// # Errors
    ///
    /// Returns `ClientError::TransactionConflict` if all slots are occupied.
    pub fn register(
        &self,
        function_code: FunctionCode,
    ) -> Result<
        (
            TransactionId,
            oneshot::Receiver<Result<OwnedResponsePdu, ClientError>>,
        ),
        ClientError,
    > {
        let mut seen_slots = [false; MAX_SLOTS];
        let mut seen_count = 0;

        while seen_count < MAX_SLOTS {
            let txn_id = self.next_transaction_id();
            let slot_idx = txn_id.0 as usize % MAX_SLOTS;
            if !seen_slots[slot_idx] {
                seen_slots[slot_idx] = true;
                seen_count += 1;
            }

            let mut slot = self.slots[slot_idx].lock();
            if slot.is_none() {
                let (tx, rx) = oneshot::channel();

                *slot = Some(PendingTransaction {
                    txn_id,
                    sender: tx,
                    sent_at: Instant::now(),
                    function_code,
                });

                return Ok((txn_id, rx));
            }
        }

        let last_id = self.next_id.load(Ordering::Relaxed).wrapping_sub(1);
        Err(ClientError::TransactionConflict(TransactionId(last_id)))
    }

    /// Complete a transaction with a response.
    ///
    /// Verifies that the stored transaction ID matches the response's ID
    /// to prevent stale responses from being delivered to the wrong caller.
    ///
    /// Returns `true` if the transaction was found and completed, `false`
    /// if the slot was empty or the ID did not match (stale response).
    pub fn complete(
        &self,
        txn_id: TransactionId,
        response: Result<OwnedResponsePdu, ClientError>,
    ) -> bool {
        let slot_idx = txn_id.0 as usize % MAX_SLOTS;
        let mut slot = self.slots[slot_idx].lock();

        let id_matches = slot.as_ref().is_some_and(|p| p.txn_id == txn_id);
        if id_matches {
            let pending = slot.take().expect("checked above");
            let _ = pending.sender.send(response);
            true
        } else {
            false
        }
    }

    /// Complete the single outstanding transaction, regardless of its ID.
    ///
    /// RTU responses carry no transaction ID, so they cannot be matched by ID.
    /// Under the RTU half-duplex single-in-flight invariant (enforced by
    /// [`ModbusClient::from_rtu_transport`](crate::ModbusClient::from_rtu_transport))
    /// there is at most one outstanding transaction; if more than one is somehow
    /// present, the oldest (earliest `sent_at`) is completed to preserve FIFO
    /// ordering. Returns `true` if a transaction was found and completed.
    pub fn complete_oldest(&self, response: Result<OwnedResponsePdu, ClientError>) -> bool {
        let mut oldest: Option<(usize, Instant)> = None;
        for (idx, slot) in self.slots.iter().enumerate() {
            if let Some(p) = slot.lock().as_ref()
                && oldest.is_none_or(|(_, t)| p.sent_at < t)
            {
                oldest = Some((idx, p.sent_at));
            }
        }
        if let Some((idx, _)) = oldest
            && let Some(pending) = self.slots[idx].lock().take()
        {
            let _ = pending.sender.send(response);
            return true;
        }
        false
    }

    /// Cancel all pending transactions with the given error.
    pub fn cancel_all(&self, make_error: impl Fn() -> ClientError) {
        for slot in &self.slots {
            let mut slot = slot.lock();
            if let Some(pending) = slot.take() {
                let _ = pending.sender.send(Err(make_error()));
            }
        }
    }

    /// Sweep for timed-out transactions.
    ///
    /// Returns the number of transactions that were timed out.
    pub fn sweep_timeouts(&self, timeout: std::time::Duration) -> usize {
        let now = Instant::now();
        let mut count = 0;

        for slot in &self.slots {
            let mut slot = slot.lock();
            let timed_out = slot
                .as_ref()
                .is_some_and(|p| now.duration_since(p.sent_at) > timeout);

            if timed_out && let Some(pending) = slot.take() {
                let _ = pending.sender.send(Err(ClientError::Timeout));
                count += 1;
            }
        }
        count
    }

    /// Number of currently pending transactions.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.slots.iter().filter(|s| s.lock().is_some()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn register(manager: &TransactionManager) -> TransactionId {
        let (txn_id, _rx) = manager
            .register(FunctionCode::ReadHoldingRegisters)
            .expect("transaction slot should be available");
        txn_id
    }

    #[test]
    fn transaction_ids_skip_zero_after_wraparound() {
        let manager = TransactionManager::new();
        manager.next_id.store(u16::MAX, Ordering::Relaxed);

        assert_eq!(register(&manager), TransactionId(u16::MAX));
        assert_eq!(register(&manager), TransactionId(1));
    }

    #[test]
    fn register_fills_ring_across_wrap_without_allocating_zero() {
        let manager = TransactionManager::new();
        manager.next_id.store(u16::MAX - 1, Ordering::Relaxed);

        let mut ids = Vec::with_capacity(MAX_SLOTS);
        for _ in 0..MAX_SLOTS {
            ids.push(register(&manager).0);
        }

        assert!(!ids.contains(&0));
        assert_eq!(manager.pending_count(), MAX_SLOTS);
        assert!(
            manager
                .register(FunctionCode::ReadHoldingRegisters)
                .is_err()
        );
    }

    #[test]
    fn register_does_not_false_conflict_when_wrap_duplicates_occupied_slots() {
        let manager = TransactionManager::new();

        for _ in 1..MAX_SLOTS {
            register(&manager);
        }
        assert_eq!(manager.pending_count(), MAX_SLOTS - 1);

        manager.next_id.store(u16::MAX - 1, Ordering::Relaxed);

        let txn_id = register(&manager);
        assert_eq!(txn_id.0 as usize % MAX_SLOTS, 0);
        assert_ne!(txn_id, TransactionId(0));
        assert_eq!(manager.pending_count(), MAX_SLOTS);
    }
}
