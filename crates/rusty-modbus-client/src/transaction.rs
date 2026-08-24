//! Transaction manager for Modbus/TCP pipelining.
//!
//! Manages up to 16 concurrent in-flight transactions, matched by Transaction ID.
//! Uses a fixed-size ring indexed by `transaction_id % MAX_SLOTS` to avoid `HashMap` overhead.

use std::sync::atomic::{AtomicU16, Ordering};

use bytes::Bytes;
use parking_lot::Mutex;
use rusty_modbus_codec::DecodeError;
use rusty_modbus_frame::OwnedResponsePdu;
use rusty_modbus_types::{FunctionCode, TransactionId, UnitId};
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::error::ClientError;

/// Maximum number of concurrent in-flight transactions.
pub(crate) const MAX_SLOTS: usize = 16;

#[derive(Clone, Copy)]
struct ExpectedResponse {
    unit_id: UnitId,
    function_code: FunctionCode,
}

/// Result of dispatching one transport response frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompletionOutcome {
    /// The frame completed the matching request with a decoded response.
    Delivered,
    /// No active transaction matched the frame.
    UnknownOrDuplicate,
    /// The frame's Unit Identifier did not match the pending request.
    UnitMismatch {
        /// Unit Identifier carried by the request.
        expected: u8,
        /// Unit Identifier carried by the response.
        got: u8,
    },
    /// The decoded response had the wrong normal or exception function code.
    FunctionRejected {
        /// Function code carried by the request.
        expected: u8,
        /// Function code carried by the response.
        got: u8,
    },
    /// The envelope matched, but its PDU could not be decoded.
    CodecRejected(DecodeError),
}

/// A pending transaction waiting for a response.
pub(crate) struct PendingTransaction {
    /// The transaction ID this slot was registered for.
    pub txn_id: TransactionId,
    /// Channel to send the response (or error) to the caller.
    pub sender: oneshot::Sender<Result<OwnedResponsePdu, ClientError>>,
    /// When the request was sent.
    pub sent_at: Instant,
    expected: ExpectedResponse,
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
    /// Dropping the receiver does not reclaim the slot; timeout sweeping or a
    /// later terminal event remains responsible for removing it.
    ///
    /// # Errors
    ///
    /// Returns `ClientError::TransactionConflict` if all slots are occupied.
    pub fn register(
        &self,
        unit_id: UnitId,
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
                    expected: ExpectedResponse {
                        unit_id,
                        function_code,
                    },
                });

                return Ok((txn_id, rx));
            }
        }

        let last_id = self.next_id.load(Ordering::Relaxed).wrapping_sub(1);
        Err(ClientError::TransactionConflict(TransactionId(last_id)))
    }

    /// Correlate a Modbus/TCP response with an exact pending transaction.
    ///
    /// Verifies that the stored transaction ID matches the response's ID
    /// to prevent stale responses from being delivered to the wrong caller.
    ///
    /// An unknown, duplicate, or same-ring response leaves every active slot
    /// unchanged. Once the transaction ID matches, Unit Identifier, decoding,
    /// and function identity errors are terminal for that request.
    pub fn complete_response(
        &self,
        txn_id: TransactionId,
        response_unit_id: UnitId,
        pdu: Bytes,
    ) -> CompletionOutcome {
        let slot_idx = txn_id.0 as usize % MAX_SLOTS;
        let mut slot = self.slots[slot_idx].lock();

        if slot.as_ref().is_none_or(|pending| pending.txn_id != txn_id) {
            return CompletionOutcome::UnknownOrDuplicate;
        }

        let pending = slot.take().expect("transaction ID matched above");
        deliver_response(pending, response_unit_id, pdu)
    }

    /// Correlate an RTU response with the oldest outstanding transaction.
    ///
    /// RTU responses carry no transaction ID, so they cannot be matched by ID.
    /// Under the RTU half-duplex single-in-flight invariant (enforced by
    /// [`ModbusClient::from_rtu_transport`](crate::ModbusClient::from_rtu_transport))
    /// there is at most one outstanding transaction; if more than one is somehow
    /// present, the oldest (earliest `sent_at`) is selected to preserve FIFO
    /// ordering. A response for another Unit Identifier is unrelated multidrop
    /// traffic and leaves that transaction pending.
    ///
    /// RTU has no transaction identifier, so a late response with the same
    /// Unit Identifier and function envelope is indistinguishable from the
    /// active request.
    pub fn complete_oldest_response(
        &self,
        response_unit_id: UnitId,
        pdu: Bytes,
    ) -> CompletionOutcome {
        let mut oldest: Option<(usize, Instant)> = None;
        for (idx, slot) in self.slots.iter().enumerate() {
            if let Some(p) = slot.lock().as_ref()
                && oldest.is_none_or(|(_, t)| p.sent_at < t)
            {
                oldest = Some((idx, p.sent_at));
            }
        }
        let Some((idx, _)) = oldest else {
            return CompletionOutcome::UnknownOrDuplicate;
        };

        let mut slot = self.slots[idx].lock();
        let Some(pending) = slot.as_ref() else {
            return CompletionOutcome::UnknownOrDuplicate;
        };
        let expected = pending.expected.unit_id.0;
        let got = response_unit_id.0;
        if expected != got {
            return CompletionOutcome::UnitMismatch { expected, got };
        }

        let pending = slot.take().expect("pending transaction checked above");
        deliver_response(pending, response_unit_id, pdu)
    }

    /// Remove one exact pending transaction after a local send failure.
    ///
    /// The caller retains and returns the transport error directly; closing the
    /// response channel here avoids constructing a response-correlation error.
    pub fn remove(&self, txn_id: TransactionId) -> bool {
        let slot_idx = txn_id.0 as usize % MAX_SLOTS;
        let mut slot = self.slots[slot_idx].lock();
        if slot.as_ref().is_none_or(|pending| pending.txn_id != txn_id) {
            return false;
        }

        slot.take().expect("transaction ID matched above");
        true
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

fn deliver_response(
    pending: PendingTransaction,
    response_unit_id: UnitId,
    pdu: Bytes,
) -> CompletionOutcome {
    let expected_unit_id = pending.expected.unit_id.0;
    let got_unit_id = response_unit_id.0;
    if expected_unit_id != got_unit_id {
        let _ = pending
            .sender
            .send(Err(ClientError::UnexpectedResponseUnitId {
                expected: expected_unit_id,
                got: got_unit_id,
            }));
        return CompletionOutcome::UnitMismatch {
            expected: expected_unit_id,
            got: got_unit_id,
        };
    }

    let response = match OwnedResponsePdu::from_pdu(pdu) {
        Ok(response) => response,
        Err(error) => {
            let _ = pending.sender.send(Err(ClientError::Codec(error)));
            return CompletionOutcome::CodecRejected(error);
        }
    };

    let expected = pending.expected.function_code.code();
    let got = response.function_code();
    if got != expected && got != (expected | 0x80) {
        let _ = pending
            .sender
            .send(Err(ClientError::UnexpectedResponse { expected, got }));
        return CompletionOutcome::FunctionRejected { expected, got };
    }

    let _ = pending.sender.send(Ok(response));
    CompletionOutcome::Delivered
}

#[cfg(test)]
mod tests {
    use super::*;

    type ResponseReceiver = oneshot::Receiver<Result<OwnedResponsePdu, ClientError>>;

    fn register_pending(
        manager: &TransactionManager,
        unit_id: u8,
        function_code: FunctionCode,
    ) -> (TransactionId, ResponseReceiver) {
        manager
            .register(UnitId(unit_id), function_code)
            .expect("transaction slot should be available")
    }

    fn register(manager: &TransactionManager) -> TransactionId {
        let (txn_id, _rx) = register_pending(manager, 1, FunctionCode::ReadHoldingRegisters);
        txn_id
    }

    fn receive(rx: ResponseReceiver) -> Result<OwnedResponsePdu, ClientError> {
        rx.blocking_recv()
            .expect("transaction sender should deliver a terminal result")
    }

    fn holding_register_response(value: u16) -> Bytes {
        let [high, low] = value.to_be_bytes();
        Bytes::from(vec![0x03, 0x02, high, low])
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
                .register(UnitId(1), FunctionCode::ReadHoldingRegisters)
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

    #[test]
    fn tcp_wrong_unit_terminates_exact_transaction() {
        let manager = TransactionManager::new();
        let (txn_id, rx) = register_pending(&manager, 1, FunctionCode::ReadHoldingRegisters);

        assert_eq!(
            manager.complete_response(txn_id, UnitId(2), Bytes::new()),
            CompletionOutcome::UnitMismatch {
                expected: 1,
                got: 2,
            }
        );
        assert!(matches!(
            receive(rx),
            Err(ClientError::UnexpectedResponseUnitId {
                expected: 1,
                got: 2
            })
        ));
        assert_eq!(manager.pending_count(), 0);
    }

    #[test]
    fn tcp_wrong_normal_function_is_terminal() {
        let manager = TransactionManager::new();
        let (txn_id, rx) = register_pending(&manager, 1, FunctionCode::ReadHoldingRegisters);

        assert_eq!(
            manager.complete_response(
                txn_id,
                UnitId(1),
                Bytes::from_static(&[0x04, 0x02, 0x00, 0x2A]),
            ),
            CompletionOutcome::FunctionRejected {
                expected: 0x03,
                got: 0x04,
            }
        );
        assert!(matches!(
            receive(rx),
            Err(ClientError::UnexpectedResponse {
                expected: 0x03,
                got: 0x04
            })
        ));
    }

    #[test]
    fn tcp_wrong_exception_function_is_terminal() {
        let manager = TransactionManager::new();
        let (txn_id, rx) = register_pending(&manager, 1, FunctionCode::ReadHoldingRegisters);

        assert_eq!(
            manager.complete_response(txn_id, UnitId(1), Bytes::from_static(&[0x84, 0x01]),),
            CompletionOutcome::FunctionRejected {
                expected: 0x03,
                got: 0x84,
            }
        );
        assert!(matches!(
            receive(rx),
            Err(ClientError::UnexpectedResponse {
                expected: 0x03,
                got: 0x84
            })
        ));
    }

    #[test]
    fn tcp_matching_exception_is_delivered() {
        let manager = TransactionManager::new();
        let (txn_id, rx) = register_pending(&manager, 1, FunctionCode::ReadHoldingRegisters);

        assert_eq!(
            manager.complete_response(txn_id, UnitId(1), Bytes::from_static(&[0x83, 0x02]),),
            CompletionOutcome::Delivered
        );
        assert!(matches!(receive(rx), Ok(OwnedResponsePdu::Exception(_))));
    }

    #[test]
    fn tcp_unknown_id_does_not_mutate_active_transaction() {
        let manager = TransactionManager::new();
        let (txn_id, rx) = register_pending(&manager, 1, FunctionCode::ReadHoldingRegisters);

        assert_eq!(
            manager.complete_response(TransactionId(txn_id.0 + 1), UnitId(1), Bytes::new(),),
            CompletionOutcome::UnknownOrDuplicate
        );
        assert_eq!(manager.pending_count(), 1);

        assert_eq!(
            manager.complete_response(txn_id, UnitId(1), holding_register_response(0x002A)),
            CompletionOutcome::Delivered
        );
        assert!(matches!(
            receive(rx),
            Ok(OwnedResponsePdu::ReadHoldingRegisters(_))
        ));
    }

    #[test]
    fn tcp_duplicate_response_does_not_mutate_other_slots() {
        let manager = TransactionManager::new();
        let (txn_id, rx) = register_pending(&manager, 1, FunctionCode::ReadHoldingRegisters);
        assert_eq!(
            manager.complete_response(txn_id, UnitId(1), holding_register_response(0x002A)),
            CompletionOutcome::Delivered
        );
        assert!(receive(rx).is_ok());

        for _ in 0..32 {
            assert_eq!(
                manager.complete_response(txn_id, UnitId(1), holding_register_response(0x0011)),
                CompletionOutcome::UnknownOrDuplicate
            );
        }
        assert_eq!(manager.pending_count(), 0);
    }

    #[test]
    fn tcp_same_ring_index_with_different_full_id_is_ignored() {
        let manager = TransactionManager::new();
        let (txn_id, rx) = register_pending(&manager, 1, FunctionCode::ReadHoldingRegisters);
        let slot_count = u16::try_from(MAX_SLOTS).expect("slot count fits in a transaction ID");
        let colliding_id = TransactionId(txn_id.0 + slot_count);

        assert_eq!(
            manager.complete_response(colliding_id, UnitId(1), holding_register_response(0x0011),),
            CompletionOutcome::UnknownOrDuplicate
        );
        assert_eq!(manager.pending_count(), 1);

        assert_eq!(
            manager.complete_response(txn_id, UnitId(1), holding_register_response(0x002A)),
            CompletionOutcome::Delivered
        );
        assert!(receive(rx).is_ok());
    }

    #[test]
    fn tcp_malformed_matching_response_delivers_codec_error() {
        let manager = TransactionManager::new();
        let (txn_id, rx) = register_pending(&manager, 1, FunctionCode::ReadHoldingRegisters);

        assert!(matches!(
            manager.complete_response(txn_id, UnitId(1), Bytes::from_static(&[0x03, 0x02, 0x00]),),
            CompletionOutcome::CodecRejected(_)
        ));
        assert!(matches!(receive(rx), Err(ClientError::Codec(_))));
        assert_eq!(manager.pending_count(), 0);
    }

    #[test]
    fn rtu_wrong_unit_is_ignored_before_valid_response() {
        let manager = TransactionManager::new();
        let (_txn_id, rx) = register_pending(&manager, 1, FunctionCode::ReadHoldingRegisters);

        assert_eq!(
            manager.complete_oldest_response(UnitId(2), Bytes::new()),
            CompletionOutcome::UnitMismatch {
                expected: 1,
                got: 2,
            }
        );
        assert_eq!(manager.pending_count(), 1);

        assert_eq!(
            manager.complete_oldest_response(UnitId(1), holding_register_response(0x002A)),
            CompletionOutcome::Delivered
        );
        assert!(matches!(
            receive(rx),
            Ok(OwnedResponsePdu::ReadHoldingRegisters(_))
        ));
    }

    #[test]
    fn rtu_wrong_function_is_terminal() {
        let manager = TransactionManager::new();
        let (_txn_id, rx) = register_pending(&manager, 1, FunctionCode::ReadHoldingRegisters);

        assert_eq!(
            manager.complete_oldest_response(
                UnitId(1),
                Bytes::from_static(&[0x04, 0x02, 0x00, 0x2A]),
            ),
            CompletionOutcome::FunctionRejected {
                expected: 0x03,
                got: 0x04,
            }
        );
        assert!(matches!(
            receive(rx),
            Err(ClientError::UnexpectedResponse {
                expected: 0x03,
                got: 0x04
            })
        ));
        assert_eq!(manager.pending_count(), 0);
    }

    #[test]
    fn rtu_response_without_active_request_is_ignored() {
        let manager = TransactionManager::new();

        assert_eq!(
            manager.complete_oldest_response(UnitId(1), holding_register_response(0x002A)),
            CompletionOutcome::UnknownOrDuplicate
        );
        assert_eq!(manager.pending_count(), 0);
    }

    #[test]
    fn direct_removal_requires_exact_full_transaction_id() {
        let manager = TransactionManager::new();
        let (txn_id, rx) = register_pending(&manager, 1, FunctionCode::ReadHoldingRegisters);
        let slot_count = u16::try_from(MAX_SLOTS).expect("slot count fits in a transaction ID");
        let colliding_id = TransactionId(txn_id.0 + slot_count);

        assert!(!manager.remove(colliding_id));
        assert_eq!(manager.pending_count(), 1);
        assert!(manager.remove(txn_id));
        assert!(rx.blocking_recv().is_err());
        assert_eq!(manager.pending_count(), 0);
    }
}
