//! `ModbusClient` — high-level async Modbus client.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_frame::OwnedResponsePdu;
use rusty_modbus_tcp::transport::{TransportConnect, TransportSink};
use rusty_modbus_tcp::{TcpConfig, TcpSink, TcpTransport};
use rusty_modbus_types::{FunctionCode, MbapHeader, UnitId};
use tokio::sync::{Semaphore, watch};
use tokio::time::{self, Duration};

use crate::config::ClientConfig;
use crate::error::ClientError;
use crate::reader;
use crate::transaction::TransactionManager;

/// High-level async Modbus client with transaction pipelining.
pub struct ModbusClient {
    sink: tokio::sync::Mutex<TcpSink>,
    txn_mgr: Arc<TransactionManager>,
    config: ClientConfig,
    connected: AtomicBool,
    semaphore: Arc<Semaphore>,
    shutdown_tx: watch::Sender<bool>,
    reader_handle: Option<tokio::task::JoinHandle<()>>,
    sweep_handle: Option<tokio::task::JoinHandle<()>>,
}

impl ModbusClient {
    /// Connect to a Modbus/TCP server.
    ///
    /// Establishes the TCP connection, spawns the background reader task
    /// and the timeout sweep task.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Transport`] if the connection fails.
    pub async fn connect(addr: SocketAddr, config: ClientConfig) -> Result<Self, ClientError> {
        let tcp_config = TcpConfig {
            connect_timeout: config.timeout,
            read_timeout: Some(config.timeout),
            write_timeout: Some(config.timeout),
            ..TcpConfig::default()
        };

        let (sink, stream) = TcpTransport::connect(tcp_config, addr).await?;

        let txn_mgr = Arc::new(TransactionManager::new());
        let semaphore = Arc::new(Semaphore::new(config.max_in_flight));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let reader_handle = reader::spawn_reader(stream, Arc::clone(&txn_mgr), shutdown_rx);

        // Spawn timeout sweep task.
        let sweep_txn_mgr = Arc::clone(&txn_mgr);
        let sweep_timeout = config.timeout;
        let sweep_handle = tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_millis(500));
            loop {
                interval.tick().await;
                sweep_txn_mgr.sweep_timeouts(sweep_timeout);
            }
        });

        Ok(Self {
            sink: tokio::sync::Mutex::new(sink),
            txn_mgr,
            config,
            connected: AtomicBool::new(true),
            semaphore,
            shutdown_tx,
            reader_handle: Some(reader_handle),
            sweep_handle: Some(sweep_handle),
        })
    }

    /// Whether the client is currently connected.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// The default unit ID for this client.
    #[must_use]
    pub fn unit_id(&self) -> UnitId {
        self.config.unit_id
    }

    /// Graceful shutdown: stop new requests, drain in-flight, cancel remaining.
    pub async fn shutdown(&self) {
        // Signal reader to stop.
        let _ = self.shutdown_tx.send(true);
        self.connected.store(false, Ordering::Relaxed);

        // Wait for in-flight transactions up to shutdown_timeout.
        let deadline = time::Instant::now() + self.config.shutdown_timeout;
        while time::Instant::now() < deadline && self.txn_mgr.pending_count() > 0 {
            time::sleep(Duration::from_millis(50)).await;
        }

        // Cancel any remaining.
        self.txn_mgr.cancel_all(|| ClientError::ShuttingDown);
    }

    /// Send a raw request PDU and await the owned response.
    ///
    /// This is the core method used by all typed request methods.
    pub(crate) async fn send_request(
        &self,
        unit_id: UnitId,
        function_code: FunctionCode,
        pdu_data: &[u8],
    ) -> Result<OwnedResponsePdu, ClientError> {
        if !self.is_connected() {
            return Err(ClientError::NotConnected);
        }

        // Acquire semaphore permit (limits concurrency).
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| ClientError::ShuttingDown)?;

        // Register transaction.
        let (txn_id, rx) = self.txn_mgr.register(function_code)?;

        // Build MBAP frame.
        let header = MbapHeader::new(
            txn_id.0,
            unit_id.0,
            u16::try_from(pdu_data.len()).unwrap_or(u16::MAX),
        );
        let frame = Frame {
            header: FrameHeader::Mbap(header),
            pdu: Bytes::copy_from_slice(pdu_data),
        };

        // Send frame.
        {
            let mut sink = self.sink.lock().await;
            sink.send(frame).await.map_err(ClientError::Transport)?;
        }

        // Await response via oneshot channel.
        match rx.await {
            Ok(result) => result,
            Err(_) => Err(ClientError::ShuttingDown),
        }
    }

    /// Send a broadcast write (Unit ID 0x00) — no response expected.
    pub(crate) async fn send_broadcast(
        &self,
        pdu_data: &[u8],
    ) -> Result<(), ClientError> {
        if !self.is_connected() {
            return Err(ClientError::NotConnected);
        }

        let header = MbapHeader::new(
            0, // Transaction ID doesn't matter for broadcast.
            0x00,
            u16::try_from(pdu_data.len()).unwrap_or(u16::MAX),
        );
        let frame = Frame {
            header: FrameHeader::Mbap(header),
            pdu: Bytes::copy_from_slice(pdu_data),
        };

        let mut sink = self.sink.lock().await;
        sink.send(frame).await.map_err(ClientError::Transport)?;

        Ok(())
    }

    /// Send a request with retry logic.
    pub(crate) async fn send_with_retry(
        &self,
        unit_id: UnitId,
        function_code: FunctionCode,
        pdu_data: &[u8],
    ) -> Result<OwnedResponsePdu, ClientError> {
        let mut last_error = None;

        for attempt in 0..=self.config.retry.max_retries {
            if attempt > 0 {
                time::sleep(self.config.retry.retry_delay).await;
            }

            match self.send_request(unit_id, function_code, pdu_data).await {
                Ok(response) => {
                    // Check if it's an exception response we should retry.
                    if let OwnedResponsePdu::Exception(exc) = response {
                        if self.config.retry.is_retryable(exc.exception_code) {
                            last_error = Some(ClientError::Exception(exc));
                            continue;
                        }
                        return Ok(OwnedResponsePdu::Exception(exc));
                    }
                    return Ok(response);
                }
                Err(ClientError::Timeout) if attempt < self.config.retry.max_retries => {
                    last_error = Some(ClientError::Timeout);
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }

        Err(ClientError::RetriesExhausted {
            attempts: self.config.retry.max_retries + 1,
            last_error: Box::new(last_error.unwrap_or(ClientError::Timeout)),
        })
    }
}

impl Drop for ModbusClient {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
        if let Some(h) = self.reader_handle.take() {
            h.abort();
        }
        if let Some(h) = self.sweep_handle.take() {
            h.abort();
        }
    }
}

impl std::fmt::Debug for ModbusClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModbusClient")
            .field("unit_id", &self.config.unit_id)
            .field("connected", &self.is_connected())
            .field("pending", &self.txn_mgr.pending_count())
            .finish_non_exhaustive()
    }
}
