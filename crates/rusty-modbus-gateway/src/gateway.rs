//! `ModbusGateway` — TCP ↔ RTU bridge.

use std::net::SocketAddr;
use std::sync::Arc;

use rusty_modbus_frame::frame::FrameHeader;
use rusty_modbus_rtu::rtu_tcp::RtuOverTcpTransport;
use rusty_modbus_tcp::TcpConfig;
use rusty_modbus_tcp::config::TcpServerConfig;
use rusty_modbus_tcp::listener::TcpServerListener;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_types::{ExceptionCode, TransactionId, UnitId};
use tokio::sync::watch;
use tokio::time;

use crate::config::GatewayConfig;
use crate::error::GatewayError;
use crate::routing::RouteTable;
use crate::translator;

/// TCP ↔ RTU bridge gateway.
pub struct ModbusGateway {
    local_addr: SocketAddr,
    shutdown_tx: watch::Sender<bool>,
    accept_handle: Option<tokio::task::JoinHandle<()>>,
}

impl ModbusGateway {
    /// Start the gateway — binds TCP listener and begins accepting connections.
    pub async fn start(config: GatewayConfig) -> Result<Self, GatewayError> {
        let tcp_config = TcpServerConfig {
            max_connections: config.max_tcp_connections,
            ..config.tcp_config.clone()
        };

        let listener = TcpServerListener::bind(config.tcp_listen, tcp_config)
            .await
            .map_err(|e| match e {
                rusty_modbus_tcp::TransportError::Io(io) => GatewayError::Bind(io),
                other => GatewayError::Transport(other),
            })?;

        let local_addr = listener.local_addr().map_err(|e| match e {
            rusty_modbus_tcp::TransportError::Io(io) => GatewayError::Bind(io),
            other => GatewayError::Transport(other),
        })?;

        let route_table = Arc::new(RouteTable::new(config.routes));
        let serial_timeout = config.serial_timeout;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let accept_handle = tokio::spawn(async move {
            accept_loop(listener, route_table, serial_timeout, shutdown_rx).await;
        });

        Ok(Self {
            local_addr,
            shutdown_tx,
            accept_handle: Some(accept_handle),
        })
    }

    /// Stop the gateway.
    pub async fn stop(&self) {
        let _ = self.shutdown_tx.send(true);
        time::sleep(std::time::Duration::from_millis(100)).await;
    }

    /// Local address the gateway TCP listener is bound to.
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

impl Drop for ModbusGateway {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
        if let Some(h) = self.accept_handle.take() {
            h.abort();
        }
    }
}

impl std::fmt::Debug for ModbusGateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModbusGateway")
            .field("addr", &self.local_addr)
            .finish_non_exhaustive()
    }
}

async fn accept_loop(
    listener: TcpServerListener,
    route_table: Arc<RouteTable>,
    serial_timeout: std::time::Duration,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            result = listener.accept() => {
                if let Ok((sink, stream, _)) = result {
                    let rt = Arc::clone(&route_table);
                    tokio::spawn(async move {
                        handle_tcp_connection(sink, stream, rt, serial_timeout).await;
                    });
                }
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
        }
    }
}

async fn handle_tcp_connection(
    mut tcp_sink: rusty_modbus_tcp::TcpSink,
    mut tcp_stream: rusty_modbus_tcp::TcpRecvStream,
    route_table: Arc<RouteTable>,
    serial_timeout: std::time::Duration,
) {
    while let Ok(frame) = tcp_stream.recv().await {
        let unit_id = frame.unit_id();
        let txn_id = match frame.header {
            FrameHeader::Mbap(h) => TransactionId(h.transaction_id.get()),
            FrameHeader::Rtu { .. } => TransactionId(0),
        };

        // Handle broadcast (Unit ID 0x00).
        if UnitId(unit_id).is_broadcast() {
            // Forward to all backends — fire and forget, no response.
            for backend in route_table.all_backends() {
                let rtu_frame = translator::mbap_to_rtu(&frame);
                let tcp_config = TcpConfig::default();
                tokio::spawn(async move {
                    if let Ok((mut sink, _stream)) =
                        RtuOverTcpTransport::connect(backend, tcp_config).await
                    {
                        let _ = sink.send(rtu_frame).await;
                    }
                });
            }
            // No response to TCP client for broadcast.
            continue;
        }

        // Route by unit ID.
        let Some(backend_addr) = route_table.resolve(unit_id) else {
            // No route → GatewayPathUnavailable (0x0A).
            let exc = translator::make_exception_frame(
                txn_id,
                unit_id,
                frame.pdu.first().copied().unwrap_or(0),
                ExceptionCode::GatewayPathUnavailable.code(),
            );
            if tcp_sink.send(exc).await.is_err() {
                break;
            }
            continue;
        };

        // Connect to backend RTU-over-TCP device.
        let tcp_config = TcpConfig {
            connect_timeout: serial_timeout,
            read_timeout: Some(serial_timeout),
            write_timeout: Some(serial_timeout),
            ..TcpConfig::default()
        };

        let rtu_frame = translator::mbap_to_rtu(&frame);
        let fc = frame.pdu.first().copied().unwrap_or(0);

        let resp_frame = forward_to_backend(
            backend_addr,
            tcp_config,
            rtu_frame,
            serial_timeout,
            txn_id,
            unit_id,
            fc,
        )
        .await;
        if tcp_sink.send(resp_frame).await.is_err() {
            break;
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn forward_to_backend(
    backend_addr: SocketAddr,
    tcp_config: TcpConfig,
    rtu_frame: rusty_modbus_frame::Frame,
    serial_timeout: std::time::Duration,
    txn_id: TransactionId,
    unit_id: u8,
    fc: u8,
) -> rusty_modbus_frame::Frame {
    let Ok((mut rtu_sink, mut rtu_stream)) =
        RtuOverTcpTransport::connect(backend_addr, tcp_config).await
    else {
        return translator::make_exception_frame(
            txn_id,
            unit_id,
            fc,
            ExceptionCode::GatewayPathUnavailable.code(),
        );
    };

    if rtu_sink.send(rtu_frame).await.is_err() {
        return translator::make_exception_frame(
            txn_id,
            unit_id,
            fc,
            ExceptionCode::GatewayPathUnavailable.code(),
        );
    }

    if let Ok(Ok(rtu_response)) = time::timeout(serial_timeout, rtu_stream.recv()).await {
        translator::rtu_to_mbap(&rtu_response, txn_id, unit_id)
    } else {
        translator::make_exception_frame(
            txn_id,
            unit_id,
            fc,
            ExceptionCode::GatewayTargetDeviceFailedToRespond.code(),
        )
    }
}
