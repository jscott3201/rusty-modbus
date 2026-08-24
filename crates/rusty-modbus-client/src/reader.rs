//! Background frame reader task.
//!
//! Continuously reads frames from the transport and dispatches them
//! to pending transactions by matching the MBAP Transaction ID.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rusty_modbus_frame::frame::FrameHeader;
use rusty_modbus_tcp::transport::TransportStream;
use rusty_modbus_types::{TransactionId, UnitId};
use tokio::sync::watch;
use tracing::{debug, trace, warn};

use crate::error::ClientError;
use crate::transaction::{CompletionOutcome, TransactionManager};

/// Spawn the background reader task.
///
/// Reads frames from the transport stream, looks up the transaction ID,
/// and completes the pending transaction. Sets `connected` to `false`
/// when the transport closes or an error occurs. Runs until the transport
/// closes or the shutdown signal is received.
pub(crate) fn spawn_reader<R: TransportStream + Send + 'static>(
    mut stream: R,
    txn_mgr: Arc<TransactionManager>,
    connected: Arc<AtomicBool>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = stream.recv() => {
                    match result {
                        Ok(frame) => {
                            let header = frame.header;
                            let pdu_len = frame.pdu.len();
                            trace!(pdu_len, "received Modbus response frame");
                            match header {
                                FrameHeader::Mbap(h) => {
                                    let txn_id = h.transaction_id.get();
                                    trace!(txn_id, pdu_len, "matching Modbus/TCP response to transaction");
                                    let outcome = txn_mgr.complete_response(
                                        TransactionId(txn_id),
                                        UnitId(h.unit_id),
                                        frame.pdu,
                                    );
                                    trace_tcp_outcome(txn_id, pdu_len, outcome);
                                }
                                FrameHeader::Rtu { unit_id } => {
                                    // RTU frames carry no transaction ID. The
                                    // client is single-in-flight for RTU (see
                                    // ModbusClient::from_rtu_transport), so match
                                    // the response to the one outstanding request.
                                    trace!(pdu_len, "matching RTU response to oldest transaction");
                                    let outcome = txn_mgr.complete_oldest_response(
                                        UnitId(unit_id),
                                        frame.pdu,
                                    );
                                    trace_rtu_outcome(unit_id, pdu_len, outcome);
                                }
                            }
                        }
                        Err(rusty_modbus_tcp::TransportError::Timeout) => {
                            // A benign idle read timeout is NOT a connection
                            // failure for a long-lived pipelined reader: the
                            // socket simply had no frame within `read_timeout`.
                            // Per-request deadlines are enforced by the
                            // transaction manager's timeout sweep; a genuinely
                            // dead peer eventually surfaces as a transport error
                            // once TCP keepalive probes fail (bounded by the
                            // keepalive time + interval set in the transport).
                            // Keep the reader alive and wait for the next frame
                            // rather than tearing down a healthy idle connection.
                            //
                            // NOTE: `is_connected()` therefore stays true on an
                            // idle — or silently half-open — socket until that
                            // keepalive-driven error arrives.
                        }
                        Err(rusty_modbus_tcp::TransportError::Disconnected) => {
                            debug!("Modbus reader observed transport disconnect");
                            connected.store(false, Ordering::Relaxed);
                            txn_mgr.cancel_all(|| ClientError::Transport(
                                rusty_modbus_tcp::TransportError::Disconnected,
                            ));
                            break;
                        }
                        Err(e) => {
                            warn!(error = %e, "Modbus reader stopped after transport error");
                            connected.store(false, Ordering::Relaxed);
                            // Preserve the actual error description for all
                            // pending callers instead of fabricating a generic Timeout.
                            let msg = e.to_string();
                            txn_mgr.cancel_all(|| ClientError::Transport(
                                rusty_modbus_tcp::TransportError::Io(std::io::Error::new(
                                    std::io::ErrorKind::ConnectionAborted,
                                    msg.clone(),
                                ))
                            ));
                            break;
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        debug!("Modbus reader received shutdown signal");
                        break;
                    }
                }
            }
        }
    })
}

fn trace_tcp_outcome(txn_id: u16, pdu_len: usize, outcome: CompletionOutcome) {
    match outcome {
        CompletionOutcome::Delivered => {
            trace!(txn_id, pdu_len, "delivered Modbus/TCP response");
        }
        CompletionOutcome::UnknownOrDuplicate => {
            trace!(
                txn_id,
                pdu_len, "ignored unknown or duplicate Modbus/TCP response"
            );
        }
        CompletionOutcome::UnitMismatch { expected, got } => {
            warn!(
                txn_id,
                expected, got, "rejected Modbus/TCP response with unexpected unit ID"
            );
        }
        CompletionOutcome::FunctionRejected { expected, got } => {
            warn!(
                txn_id,
                expected, got, "rejected Modbus/TCP response with unexpected function code"
            );
        }
        CompletionOutcome::CodecRejected(error) => {
            warn!(
                txn_id,
                error = %error,
                pdu_len,
                "failed to decode matching Modbus/TCP response PDU"
            );
        }
    }
}

fn trace_rtu_outcome(unit_id: u8, pdu_len: usize, outcome: CompletionOutcome) {
    match outcome {
        CompletionOutcome::Delivered => {
            trace!(unit_id, pdu_len, "delivered RTU response");
        }
        CompletionOutcome::UnknownOrDuplicate => {
            trace!(
                unit_id,
                pdu_len, "ignored RTU response with no active request"
            );
        }
        CompletionOutcome::UnitMismatch { expected, got } => {
            trace!(
                expected,
                got, pdu_len, "ignored RTU response for another unit"
            );
        }
        CompletionOutcome::FunctionRejected { expected, got } => {
            warn!(
                unit_id,
                expected, got, "rejected RTU response with unexpected function code"
            );
        }
        CompletionOutcome::CodecRejected(error) => {
            warn!(
                unit_id,
                error = %error,
                pdu_len,
                "failed to decode matching RTU response PDU"
            );
        }
    }
}
