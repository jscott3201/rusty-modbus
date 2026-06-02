//! `ModbusClient` — high-level async Modbus client.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use rusty_modbus_frame::FrameError;
use rusty_modbus_frame::OwnedResponsePdu;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_tcp::transport::{TransportConnect, TransportSink, TransportStream};
use rusty_modbus_tcp::{TcpConfig, TcpSink, TcpTransport, TransportError};
use rusty_modbus_types::{FunctionCode, MAX_PDU_SIZE, MbapHeader, UnitId};
use tokio::sync::{Semaphore, watch};
use tokio::time::{self, Duration};
use tracing::{debug, trace, warn};

use crate::config::ClientConfig;
use crate::error::ClientError;
use crate::reader;
use crate::transaction::{self, TransactionManager};

/// High-level async Modbus client with transaction pipelining.
pub struct ModbusClient<S: TransportSink + Send + 'static = TcpSink> {
    sink: tokio::sync::Mutex<S>,
    txn_mgr: Arc<TransactionManager>,
    config: ClientConfig,
    connected: Arc<AtomicBool>,
    semaphore: Arc<Semaphore>,
    shutdown_tx: watch::Sender<bool>,
    reader_handle: Option<tokio::task::JoinHandle<()>>,
    sweep_handle: Option<tokio::task::JoinHandle<()>>,
}

fn checked_pdu_length(pdu_len: usize) -> Result<u16, ClientError> {
    if pdu_len == 0 {
        return Err(ClientError::Transport(TransportError::Frame(
            FrameError::InvalidPduLength {
                length: pdu_len,
                minimum: 1,
            },
        )));
    }
    if pdu_len > MAX_PDU_SIZE {
        return Err(ClientError::Transport(TransportError::Frame(
            FrameError::PduLengthOverflow {
                length: pdu_len,
                maximum: MAX_PDU_SIZE,
            },
        )));
    }

    Ok(u16::try_from(pdu_len).expect("MAX_PDU_SIZE fits in u16"))
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
    #[tracing::instrument(level = "debug", skip(config), fields(addr = %addr, timeout = ?config.timeout))]
    pub async fn connect(addr: SocketAddr, config: ClientConfig) -> Result<Self, ClientError> {
        let tcp_config = TcpConfig {
            connect_timeout: config.timeout,
            read_timeout: Some(config.timeout),
            write_timeout: Some(config.timeout),
            ..TcpConfig::default()
        };

        debug!("connecting Modbus/TCP client");
        let (sink, stream) = TcpTransport::connect(tcp_config, addr).await?;
        debug!("Modbus/TCP client connected");

        Ok(Self::from_transport(sink, stream, config))
    }
}

impl<S: TransportSink + Send + 'static> ModbusClient<S> {
    /// Create a client from pre-connected transport halves.
    ///
    /// This is the generic constructor used by [`connect()`](ModbusClient::connect)
    /// and by TLS transports that establish their own connection.
    ///
    /// `max_in_flight` is clamped to `1..=16` to match the fixed-size
    /// transaction ring.
    pub fn from_transport<R: TransportStream + Send + 'static>(
        sink: S,
        stream: R,
        config: ClientConfig,
    ) -> Self {
        let txn_mgr = Arc::new(TransactionManager::new());
        let max_in_flight = config.max_in_flight.clamp(1, transaction::MAX_SLOTS);
        let semaphore = Arc::new(Semaphore::new(max_in_flight));
        let connected = Arc::new(AtomicBool::new(true));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        debug!(
            max_in_flight,
            timeout = ?config.timeout,
            shutdown_timeout = ?config.shutdown_timeout,
            "initializing Modbus client transport"
        );

        let reader_handle = reader::spawn_reader(
            stream,
            Arc::clone(&txn_mgr),
            Arc::clone(&connected),
            shutdown_rx,
        );

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

        Self {
            sink: tokio::sync::Mutex::new(sink),
            txn_mgr,
            config,
            connected,
            semaphore,
            shutdown_tx,
            reader_handle: Some(reader_handle),
            sweep_handle: Some(sweep_handle),
        }
    }

    /// Create a client over an RTU transport (RTU-over-TCP or serial).
    ///
    /// RTU is half-duplex: exactly one request may be outstanding at a time, and
    /// RTU frames carry no transaction ID. This constructor therefore forces
    /// `max_in_flight = 1`, and the reader matches each RTU response to the
    /// single outstanding request. Use [`from_transport`](Self::from_transport)
    /// for Modbus/TCP, which supports full 16-slot pipelining.
    ///
    /// This fixes RTU request/response correlation. Serial-line framing timing
    /// (t3.5/t1.5) is a separate concern handled by the serial transport.
    pub fn from_rtu_transport<R: TransportStream + Send + 'static>(
        sink: S,
        stream: R,
        mut config: ClientConfig,
    ) -> Self {
        config.max_in_flight = 1;
        Self::from_transport(sink, stream, config)
    }

    /// Whether the client is currently connected.
    ///
    /// Reflects graceful close (peer FIN) and transport errors surfaced by the
    /// background reader — it is **not** an active liveness probe. A benign idle
    /// read timeout does not flip this to `false`, so a silently half-open
    /// socket (e.g. a peer crash without RST) may report connected until TCP
    /// keepalive probes fail. In-flight requests still fail with
    /// [`ClientError::Timeout`] via the transaction sweep regardless.
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
        debug!(
            pending = self.txn_mgr.pending_count(),
            "shutting down Modbus client"
        );
        // Signal reader to stop.
        let _ = self.shutdown_tx.send(true);
        self.connected.store(false, Ordering::Relaxed);

        // Wait for in-flight transactions up to shutdown_timeout.
        let deadline = time::Instant::now() + self.config.shutdown_timeout;
        while time::Instant::now() < deadline && self.txn_mgr.pending_count() > 0 {
            time::sleep(Duration::from_millis(50)).await;
        }

        // Cancel any remaining.
        let remaining = self.txn_mgr.pending_count();
        if remaining > 0 {
            warn!(
                pending = remaining,
                "cancelling pending Modbus transactions"
            );
        }
        self.txn_mgr.cancel_all(|| ClientError::ShuttingDown);
    }

    /// Send a raw request PDU and await the owned response.
    ///
    /// This is the core method used by all typed request methods.
    #[tracing::instrument(
        level = "debug",
        skip(self, pdu_data),
        fields(
            unit_id = unit_id.0,
            function_code = function_code.code(),
            pdu_len = pdu_data.len(),
            txn_id = tracing::field::Empty,
        )
    )]
    pub(crate) async fn send_request(
        &self,
        unit_id: UnitId,
        function_code: FunctionCode,
        pdu_data: &[u8],
    ) -> Result<OwnedResponsePdu, ClientError> {
        if !self.is_connected() {
            warn!("request rejected because client is disconnected");
            return Err(ClientError::NotConnected);
        }
        let pdu_len = checked_pdu_length(pdu_data.len())?;

        // Acquire semaphore permit (limits concurrency).
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| ClientError::ShuttingDown)?;

        // Register transaction.
        let (txn_id, rx) = self.txn_mgr.register(function_code)?;
        tracing::Span::current().record("txn_id", txn_id.0);
        trace!(txn_id = txn_id.0, "registered Modbus transaction");

        // Build MBAP frame.
        let header = MbapHeader::new(txn_id.0, unit_id.0, pdu_len);
        let frame = Frame {
            header: FrameHeader::Mbap(header),
            pdu: Bytes::copy_from_slice(pdu_data),
        };

        // Send frame. If send fails, cancel the registered transaction to
        // prevent the slot from being leaked until the sweep timeout.
        {
            let mut sink = self.sink.lock().await;
            if let Err(e) = sink.send(frame).await {
                warn!(txn_id = txn_id.0, error = %e, "failed to send Modbus request");
                self.txn_mgr
                    .complete(txn_id, Err(ClientError::Transport(e)));
                return match rx.await {
                    Ok(result) => result,
                    Err(_) => Err(ClientError::ShuttingDown),
                };
            }
        }
        trace!(txn_id = txn_id.0, "sent Modbus request frame");

        // Await response via oneshot channel.
        let response = if let Ok(result) = rx.await {
            result?
        } else {
            warn!(
                txn_id = txn_id.0,
                "response channel closed before completion"
            );
            return Err(ClientError::ShuttingDown);
        };

        // Verify the server echoed the requested function code (Modbus V1.1b3
        // §4.4: a normal response repeats the FC; an error response sets
        // `fc | 0x80`). Reject anything else so a stale or misrouted frame that
        // happens to land on this transaction slot cannot be delivered as if it
        // answered this request.
        let expected = function_code.code();
        let got = response.function_code();
        if got != expected && got != (expected | 0x80) {
            warn!(
                txn_id = txn_id.0,
                expected, got, "server response function code did not match request"
            );
            return Err(ClientError::UnexpectedResponse { expected, got });
        }

        debug!(
            txn_id = txn_id.0,
            response_function_code = got,
            "received Modbus response"
        );
        Ok(response)
    }

    /// Send a broadcast write (Unit ID 0x00) — no response expected.
    #[tracing::instrument(
        level = "debug",
        skip(self, pdu_data),
        fields(unit_id = 0u8, pdu_len = pdu_data.len())
    )]
    pub(crate) async fn send_broadcast(&self, pdu_data: &[u8]) -> Result<(), ClientError> {
        if !self.is_connected() {
            warn!("broadcast rejected because client is disconnected");
            return Err(ClientError::NotConnected);
        }
        let pdu_len = checked_pdu_length(pdu_data.len())?;

        let header = MbapHeader::new(
            0, // Transaction ID doesn't matter for broadcast.
            0x00, pdu_len,
        );
        let frame = Frame {
            header: FrameHeader::Mbap(header),
            pdu: Bytes::copy_from_slice(pdu_data),
        };

        let mut sink = self.sink.lock().await;
        sink.send(frame).await.map_err(ClientError::Transport)?;
        debug!("sent Modbus broadcast frame");

        Ok(())
    }

    /// Send a request with retry logic.
    #[tracing::instrument(
        level = "debug",
        skip(self, pdu_data),
        fields(
            unit_id = unit_id.0,
            function_code = function_code.code(),
            max_retries = self.config.retry.max_retries
        )
    )]
    pub(crate) async fn send_with_retry(
        &self,
        unit_id: UnitId,
        function_code: FunctionCode,
        pdu_data: &[u8],
    ) -> Result<OwnedResponsePdu, ClientError> {
        let mut last_error = None;

        for attempt in 0..=self.config.retry.max_retries {
            if attempt > 0 {
                debug!(attempt, "retrying Modbus request");
                time::sleep(self.config.retry.retry_delay).await;
            }

            match self.send_request(unit_id, function_code, pdu_data).await {
                Ok(response) => {
                    // Check if it's an exception response we should retry.
                    if let OwnedResponsePdu::Exception(exc) = response {
                        if self.config.retry.is_retryable(exc.exception_code) {
                            warn!(
                                attempt,
                                exception_code = exc.exception_code.code(),
                                "received retryable Modbus exception"
                            );
                            last_error = Some(ClientError::Exception(exc));
                            continue;
                        }
                        return Ok(OwnedResponsePdu::Exception(exc));
                    }
                    return Ok(response);
                }
                Err(ClientError::Timeout) if attempt < self.config.retry.max_retries => {
                    warn!(attempt, "Modbus request timed out; retrying");
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

impl<S: TransportSink + Send + 'static> Drop for ModbusClient<S> {
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

impl<S: TransportSink + Send + 'static> std::fmt::Debug for ModbusClient<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModbusClient")
            .field("unit_id", &self.config.unit_id)
            .field("connected", &self.is_connected())
            .field("pending", &self.txn_mgr.pending_count())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_pdu_length_accepts_modbus_bounds() {
        assert_eq!(checked_pdu_length(1).unwrap(), 1);
        assert_eq!(checked_pdu_length(MAX_PDU_SIZE).unwrap(), 253);
    }

    #[test]
    fn checked_pdu_length_rejects_empty_pdu() {
        assert!(matches!(
            checked_pdu_length(0),
            Err(ClientError::Transport(TransportError::Frame(
                FrameError::InvalidPduLength {
                    length: 0,
                    minimum: 1,
                }
            )))
        ));
    }

    #[test]
    fn checked_pdu_length_rejects_oversized_pdu() {
        assert!(matches!(
            checked_pdu_length(MAX_PDU_SIZE + 1),
            Err(ClientError::Transport(TransportError::Frame(
                FrameError::PduLengthOverflow {
                    length,
                    maximum: MAX_PDU_SIZE,
                }
            ))) if length == MAX_PDU_SIZE + 1
        ));
    }
}
