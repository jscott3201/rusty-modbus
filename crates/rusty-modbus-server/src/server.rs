//! `ModbusServer` — async Modbus server with pluggable data store.

use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_tcp::config::TcpServerConfig;
use rusty_modbus_tcp::listener::TcpServerListener;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_types::{MbapHeader, UnitId};
use tokio::sync::watch;

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
                if let Ok((sink, stream, _addr, guard)) = result {
                    let conn_store = Arc::clone(&store);
                    let conn_device_id = device_id.clone();
                    tokio::spawn(async move {
                        handle_connection(sink, stream, unit_id, conn_store, conn_device_id).await;
                        drop(guard);
                    });
                }
                // Accept error — could be transient; continue.
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
        }
    }
}

async fn handle_connection<S: DataStore>(
    mut sink: rusty_modbus_tcp::TcpSink,
    mut stream: rusty_modbus_tcp::TcpRecvStream,
    unit_id: UnitId,
    store: Arc<S>,
    device_id: DeviceIdentification,
) {
    while let Ok(frame) = stream.recv().await {
        let request_unit_id = UnitId(frame.unit_id());

        // Check unit ID: accept if it matches, is broadcast (0x00), or is TCP direct (0xFF).
        if request_unit_id.0 != unit_id.0
            && !request_unit_id.is_broadcast()
            && !request_unit_id.is_tcp_device()
        {
            // Not for us — discard silently.
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
            let header = MbapHeader::new(
                txn_id,
                request_unit_id.0,
                u16::try_from(response_pdu.len()).unwrap_or(u16::MAX),
            );
            let resp_frame = Frame {
                header: FrameHeader::Mbap(header),
                pdu: Bytes::from(response_pdu),
            };
            if sink.send(resp_frame).await.is_err() {
                break; // Connection lost.
            }
        }
        // If process_request returned None, it was a broadcast — no response.
    }
}
