//! Transaction manager for Modbus/TCP pipelining.
//!
//! Manages up to 16 concurrent in-flight transactions, matched by Transaction ID.
//! Uses a fixed-size ring indexed by `transaction_id % MAX_SLOTS` to avoid `HashMap` overhead.

use std::sync::atomic::{AtomicU16, Ordering};

use modbus_frame::OwnedResponsePdu;
use modbus_types::{FunctionCode, TransactionId};
use parking_lot::Mutex;
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::error::ClientError;

/// Maximum number of concurrent in-flight transactions.
const MAX_SLOTS: usize = 16;

/// A pending transaction waiting for a response.
pub(crate) struct PendingTransaction {
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
            next_id: AtomicU16::new(1), // Start at 1; 0 is sometimes special.
            slots: std::array::from_fn(|_| Mutex::new(None)),
        }
    }

    /// Allocate a new transaction ID and register a pending transaction.
    ///
    /// Returns the transaction ID and a receiver that will yield the response.
    ///
    /// # Errors
    ///
    /// Returns `ClientError::TransactionConflict` if the slot is already occupied.
    pub fn register(
        &self,
        function_code: FunctionCode,
    ) -> Result<(TransactionId, oneshot::Receiver<Result<OwnedResponsePdu, ClientError>>), ClientError>
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let txn_id = TransactionId(id);
        let slot_idx = id as usize % MAX_SLOTS;

        let (tx, rx) = oneshot::channel();

        let pending = PendingTransaction {
            sender: tx,
            sent_at: Instant::now(),
            function_code,
        };

        let mut slot = self.slots[slot_idx].lock();
        if slot.is_some() {
            return Err(ClientError::TransactionConflict(txn_id));
        }
        *slot = Some(pending);

        Ok((txn_id, rx))
    }

    /// Complete a transaction with a response.
    ///
    /// Returns `true` if the transaction was found and completed, `false` if not found.
    pub fn complete(&self, txn_id: TransactionId, response: Result<OwnedResponsePdu, ClientError>) -> bool {
        let slot_idx = txn_id.0 as usize % MAX_SLOTS;
        let mut slot = self.slots[slot_idx].lock();

        if let Some(pending) = slot.take() {
            let _ = pending.sender.send(response);
            true
        } else {
            false
        }
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

            if timed_out {
                if let Some(pending) = slot.take() {
                    let _ = pending.sender.send(Err(ClientError::Timeout));
                    count += 1;
                }
            }
        }
        count
    }

    /// Number of currently pending transactions.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| s.lock().is_some())
            .count()
    }
}
