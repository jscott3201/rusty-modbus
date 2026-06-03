//! `ModbusServer` — async Modbus server with pluggable data store.

use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_tcp::config::TcpServerConfig;
use rusty_modbus_tcp::listener::TcpServerListener;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_types::{ExceptionCode, MAX_PDU_SIZE, MbapHeader, UnitId};
use tokio::sync::watch;
use tracing::{debug, info, trace, warn};

use crate::config::{DeviceIdentification, ServerConfig};
use crate::error::ServerError;
use crate::handler;
use crate::store::DataStore;

/// Async Modbus server, generic over the data store implementation.
pub struct ModbusServer<S: DataStore> {
    config: ServerConfig,
    store: Arc<S>,
    local_addr: SocketAddr,
    shutdown_tx: watch::Sender<bool>,
    accept_handle: Option<tokio::task::JoinHandle<()>>,
}

impl<S: DataStore + 'static> ModbusServer<S> {
    /// Create and start a new Modbus server.
    ///
    /// Binds to the configured address and begins accepting connections immediately.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Bind`] if the address cannot be bound.
    #[tracing::instrument(level = "debug", skip(config, store), fields(addr = %config.listen_addr, unit_id = config.unit_id.0))]
    pub async fn start(config: ServerConfig, store: Arc<S>) -> Result<Self, ServerError> {
        let tcp_config = TcpServerConfig {
            max_connections: config.max_connections,
            ..config.tcp_config.clone()
        };

        let listener = TcpServerListener::bind(config.listen_addr, tcp_config)
            .await
            .map_err(|e| match e {
                rusty_modbus_tcp::TransportError::Io(io) => ServerError::Bind(io),
                other => ServerError::Transport(other),
            })?;

        let local_addr = listener.local_addr().map_err(|e| match e {
            rusty_modbus_tcp::TransportError::Io(io) => ServerError::Bind(io),
            other => ServerError::Transport(other),
        })?;
        info!(addr = %local_addr, unit_id = config.unit_id.0, "Modbus server listening");

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let server_unit_id = config.unit_id;
        let server_store = Arc::clone(&store);
        let server_device_id = config.device_id.clone();

        let accept_handle = tokio::spawn(async move {
            accept_loop(
                listener,
                server_unit_id,
                server_store,
                server_device_id,
                shutdown_rx,
            )
            .await;
        });

        Ok(Self {
            config,
            store,
            local_addr,
            shutdown_tx,
            accept_handle: Some(accept_handle),
        })
    }

    /// Graceful shutdown: stop accepting, wait for in-flight, close connections.
    pub async fn stop(&self) {
        info!(addr = %self.local_addr, "stopping Modbus server");
        let _ = self.shutdown_tx.send(true);
        // Give in-flight requests time to complete.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    /// Get a reference to the data store.
    #[must_use]
    pub fn store(&self) -> &S {
        self.store.as_ref()
    }

    /// Local address the server is bound to.
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

impl<S: DataStore> Drop for ModbusServer<S> {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
        if let Some(h) = self.accept_handle.take() {
            h.abort();
        }
    }
}

impl<S: DataStore> std::fmt::Debug for ModbusServer<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModbusServer")
            .field("addr", &self.local_addr)
            .field("unit_id", &self.config.unit_id)
            .finish_non_exhaustive()
    }
}

async fn accept_loop<S: DataStore + 'static>(
    listener: TcpServerListener,
    unit_id: UnitId,
    store: Arc<S>,
    device_id: DeviceIdentification,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            result = listener.accept() => {
                if let Ok((sink, stream, addr, guard)) = result {
                    debug!(peer_addr = %addr, "accepted Modbus server connection");
                    let conn_store = Arc::clone(&store);
                    let conn_device_id = device_id.clone();
                    tokio::spawn(async move {
                        handle_connection(sink, stream, addr, unit_id, conn_store, conn_device_id).await;
                        drop(guard);
                    });
                } else if let Err(error) = result {
                    warn!(error = %error, "Modbus server accept failed");
                }
                // Accept error could be transient; continue.
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    debug!("Modbus server accept loop received shutdown");
                    break;
                }
            }
        }
    }
}

async fn handle_connection<S: DataStore>(
    mut sink: rusty_modbus_tcp::TcpSink,
    mut stream: rusty_modbus_tcp::TcpRecvStream,
    peer_addr: SocketAddr,
    unit_id: UnitId,
    store: Arc<S>,
    device_id: DeviceIdentification,
) {
    while let Ok(frame) = stream.recv().await {
        let request_unit_id = UnitId(frame.unit_id());
        let pdu_len = frame.pdu.len();
        trace!(
            peer_addr = %peer_addr,
            request_unit_id = request_unit_id.0,
            pdu_len,
            "received Modbus server request"
        );

        // Check unit ID: accept if it matches, is broadcast (0x00), or is TCP direct (0xFF).
        if request_unit_id.0 != unit_id.0
            && !request_unit_id.is_broadcast()
            && !request_unit_id.is_tcp_device()
        {
            // Not for us — discard silently.
            debug!(
                peer_addr = %peer_addr,
                request_unit_id = request_unit_id.0,
                server_unit_id = unit_id.0,
                "discarding request for different unit id"
            );
            continue;
        }

        let txn_id = match frame.header {
            FrameHeader::Mbap(h) => h.transaction_id.get(),
            FrameHeader::Rtu { .. } => 0,
        };

        // Process the request.
        if let Some(response_pdu) =
            handler::process_request(&frame.pdu, request_unit_id, store.as_ref(), &device_id).await
        {
            let Some(response_frame) = response_frame(txn_id, request_unit_id, response_pdu) else {
                warn!(peer_addr = %peer_addr, txn_id, "dropping empty Modbus response PDU");
                break;
            };
            if let Err(error) = sink.send(response_frame).await {
                debug!(peer_addr = %peer_addr, txn_id, error = %error, "failed to send Modbus response");
                break; // Connection lost.
            }
            trace!(peer_addr = %peer_addr, txn_id, "sent Modbus server response");
        }
        // If process_request returned None, it was a broadcast — no response.
    }
    debug!(peer_addr = %peer_addr, "Modbus server connection closed");
}

fn response_frame(txn_id: u16, unit_id: UnitId, response_pdu: Vec<u8>) -> Option<Frame> {
    let pdu = bounded_response_pdu(response_pdu)?;
    let pdu_len = u16::try_from(pdu.len()).expect("MAX_PDU_SIZE fits in u16");
    let header = MbapHeader::new(txn_id, unit_id.0, pdu_len);
    Some(Frame {
        header: FrameHeader::Mbap(header),
        pdu: Bytes::from(pdu),
    })
}

fn bounded_response_pdu(response_pdu: Vec<u8>) -> Option<Vec<u8>> {
    let fc = response_pdu.first().copied()?;
    if response_pdu.len() <= MAX_PDU_SIZE {
        return Some(response_pdu);
    }

    warn!(
        function_code = fc,
        pdu_len = response_pdu.len(),
        max_pdu_size = MAX_PDU_SIZE,
        "server response exceeded Modbus PDU limit"
    );
    Some(vec![fc | 0x80, ExceptionCode::ServerDeviceFailure.code()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_frame_preserves_valid_pdu() {
        let frame = response_frame(0x1234, UnitId(7), vec![0x03, 0x02, 0xAA, 0xBB])
            .expect("valid response should produce a frame");

        match frame.header {
            FrameHeader::Mbap(header) => {
                assert_eq!(header.transaction_id.get(), 0x1234);
                assert_eq!(header.unit_id, 7);
                assert_eq!(header.pdu_length(), 4);
            }
            FrameHeader::Rtu { .. } => panic!("expected MBAP response"),
        }
        assert_eq!(frame.pdu.as_ref(), &[0x03, 0x02, 0xAA, 0xBB]);
    }

    #[test]
    fn response_frame_turns_oversized_pdu_into_exception() {
        let frame = response_frame(0xBEEF, UnitId(2), vec![0x03; MAX_PDU_SIZE + 1])
            .expect("oversized response should become an exception frame");

        match frame.header {
            FrameHeader::Mbap(header) => {
                assert_eq!(header.transaction_id.get(), 0xBEEF);
                assert_eq!(header.unit_id, 2);
                assert_eq!(header.pdu_length(), 2);
            }
            FrameHeader::Rtu { .. } => panic!("expected MBAP response"),
        }
        assert_eq!(
            frame.pdu.as_ref(),
            &[0x83, ExceptionCode::ServerDeviceFailure.code()]
        );
    }

    #[test]
    fn response_frame_drops_empty_pdu() {
        assert!(response_frame(0, UnitId(1), Vec::new()).is_none());
    }
}
