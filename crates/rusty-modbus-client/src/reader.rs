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
                            let txn_id = match frame.header {
                                FrameHeader::Mbap(h) => TransactionId(h.transaction_id.get()),
                                FrameHeader::Rtu { .. } => {
                                    // RTU doesn't have transaction IDs; use slot 0.
                                    TransactionId(0)
                                }
                            };

                            // Decode into owned response.
                            match OwnedResponsePdu::from_pdu(frame.pdu) {
                                Ok(response) => {
                                    txn_mgr.complete(txn_id, Ok(response));
                                }
                                Err(e) => {
                                    txn_mgr.complete(txn_id, Err(ClientError::Codec(e)));
                                }
                            }
                        }
                        Err(rusty_modbus_tcp::TransportError::Timeout) => {
                            // A benign idle read timeout is NOT a connection
                            // failure for a long-lived pipelined reader: the
                            // socket simply had no frame within `read_timeout`.
                            // Per-request deadlines are enforced by the
                            // transaction manager's timeout sweep, and dead
                            // peers are detected via TCP keepalive. Keep the
                            // reader alive and wait for the next frame, rather
                            // than tearing down a healthy idle connection.
                        }
                        Err(rusty_modbus_tcp::TransportError::Disconnected) => {
                            connected.store(false, Ordering::Relaxed);
                            txn_mgr.cancel_all(|| ClientError::Transport(
                                rusty_modbus_tcp::TransportError::Disconnected,
                            ));
                            break;
                        }
                        Err(e) => {
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
                        break;
                    }
                }
            }
        }
    })
}
