//! Background frame reader task.
//!
//! Continuously reads frames from the transport and dispatches them
//! to pending transactions by matching the MBAP Transaction ID.

use std::sync::Arc;

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
/// and completes the pending transaction. Runs until the transport closes
/// or the shutdown signal is received.
pub(crate) fn spawn_reader<R: TransportStream + Send + 'static>(
    mut stream: R,
    txn_mgr: Arc<TransactionManager>,
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
                        Err(rusty_modbus_tcp::TransportError::Disconnected) => {
                            // Connection closed — cancel all pending.
                            txn_mgr.cancel_all(|| ClientError::Transport(
                                rusty_modbus_tcp::TransportError::Disconnected,
                            ));
                            break;
                        }
                        Err(e) => {
                            // Transport error — cancel all pending.
                            txn_mgr.cancel_all(|| ClientError::Transport(
                                rusty_modbus_tcp::TransportError::Timeout, // generic
                            ));
                            let _ = e;
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
