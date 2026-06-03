//! Background frame reader task.
//!
//! Continuously reads frames from the transport and dispatches them
//! to pending transactions by matching the MBAP Transaction ID.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rusty_modbus_frame::OwnedResponsePdu;
use rusty_modbus_frame::frame::FrameHeader;
use rusty_modbus_tcp::transport::TransportStream;
use tokio::sync::watch;
use tracing::{debug, trace, warn};

use crate::error::ClientError;
use crate::transaction::TransactionManager;
use rusty_modbus_types::TransactionId;

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
                            let result = OwnedResponsePdu::from_pdu(frame.pdu).map_err(|err| {
                                warn!(error = %err, pdu_len, "failed to decode Modbus response PDU");
                                ClientError::Codec(err)
                            });
                            match header {
                                FrameHeader::Mbap(h) => {
                                    let txn_id = h.transaction_id.get();
                                    trace!(txn_id, pdu_len, "matching Modbus/TCP response to transaction");
                                    txn_mgr.complete(
                                        TransactionId(txn_id),
                                        result,
                                    );
                                }
                                FrameHeader::Rtu { .. } => {
                                    // RTU frames carry no transaction ID. The
                                    // client is single-in-flight for RTU (see
                                    // ModbusClient::from_rtu_transport), so match
                                    // the response to the one outstanding request.
                                    trace!(pdu_len, "matching RTU response to oldest transaction");
                                    txn_mgr.complete_oldest(result);
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
