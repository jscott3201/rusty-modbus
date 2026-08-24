//! `ModbusClient` — high-level async Modbus client.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use rusty_modbus_frame::FrameError;
use rusty_modbus_frame::OwnedResponsePdu;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_tcp::transport::{TransportConnect, TransportSink, TransportStream};
use rusty_modbus_tcp::{TcpConfig, TcpSink, TcpTransport, TransportError};
use rusty_modbus_types::{ExceptionCode, FunctionCode, MAX_PDU_SIZE, MbapHeader, UnitId};
use tokio::sync::{Semaphore, watch};
use tokio::time::{self, Duration};
use tracing::{debug, trace, warn};

use crate::config::ClientConfig;
use crate::error::ClientError;
use crate::reader;
use crate::transaction::{self, TransactionManager};

/// Whether replaying a request after an ambiguous local failure is safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestKind {
    ReplaySafe,
    Mutating,
}

impl RequestKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ReplaySafe => "replay_safe",
            Self::Mutating => "mutating",
        }
    }
}

enum AttemptDisposition {
    Complete(Result<OwnedResponsePdu, ClientError>),
    Retry {
        reason: &'static str,
        error: ClientError,
    },
}

/// High-level async Modbus client with transaction pipelining.
pub struct ModbusClient<S: TransportSink + Send + 'static = TcpSink> {
    sink: tokio::sync::Mutex<S>,
    txn_mgr: Arc<TransactionManager>,
    config: ClientConfig,
    connected: Arc<AtomicBool>,
    semaphore: Arc<Semaphore>,
    shutdown_tx: watch::Sender<bool>,
    reader_handle: Option<tokio::task::JoinHandle<()>>,
    deadline_handle: Option<tokio::task::JoinHandle<()>>,
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

fn operation_budget(timeout: Duration, retry_delay: Duration, max_retries: u32) -> Duration {
    timeout
        .saturating_mul(max_retries)
        .saturating_add(timeout)
        .saturating_add(retry_delay.saturating_mul(max_retries))
}

fn saturating_instant_add(start: time::Instant, duration: Duration) -> time::Instant {
    if let Some(deadline) = start.checked_add(duration) {
        return deadline;
    }

    let mut lower = Duration::ZERO;
    let mut upper = duration;
    while upper.saturating_sub(lower) > Duration::from_nanos(1) {
        let midpoint = lower.saturating_add(upper.saturating_sub(lower) / 2);
        if start.checked_add(midpoint).is_some() {
            lower = midpoint;
        } else {
            upper = midpoint;
        }
    }
    start.checked_add(lower).unwrap_or(start)
}

async fn poll_before_deadline<F: Future>(deadline: time::Instant, future: F) -> Option<F::Output> {
    let sleep = time::sleep_until(deadline);
    tokio::pin!(sleep);
    tokio::pin!(future);
    tokio::select! {
        biased;
        () = &mut sleep => None,
        output = &mut future => Some(output),
    }
}

impl ModbusClient {
    /// Connect to a Modbus/TCP server.
    ///
    /// Establishes the TCP connection, spawns the background reader task
    /// and the request-deadline task.
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

        let deadline_txn_mgr = Arc::clone(&txn_mgr);
        let deadline_handle = tokio::spawn(async move {
            deadline_txn_mgr.run_deadline_scheduler().await;
        });

        Self {
            sink: tokio::sync::Mutex::new(sink),
            txn_mgr,
            config,
            connected,
            semaphore,
            shutdown_tx,
            reader_handle: Some(reader_handle),
            deadline_handle: Some(deadline_handle),
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
    /// [`ClientError::Timeout`] via the transaction deadline regardless.
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
        pdu_len: u16,
        attempt_deadline: time::Instant,
    ) -> Result<OwnedResponsePdu, ClientError> {
        if !self.is_connected() {
            warn!("request rejected because client is disconnected");
            return Err(ClientError::NotConnected);
        }

        // Lock before registration so a request cannot expire while queued for
        // the sink and then be transmitted as an orphan.
        let Some(mut sink) = poll_before_deadline(attempt_deadline, self.sink.lock()).await else {
            warn!("request deadline elapsed while waiting for the transport sink");
            return Err(ClientError::Timeout);
        };

        // Register transaction.
        let (txn_id, rx) = self
            .txn_mgr
            .register(unit_id, function_code, attempt_deadline)?;
        tracing::Span::current().record("txn_id", txn_id.0);
        trace!(txn_id = txn_id.0, "registered Modbus transaction");

        // Build MBAP frame.
        let header = MbapHeader::new(txn_id.0, unit_id.0, pdu_len);
        let frame = Frame {
            header: FrameHeader::Mbap(header),
            pdu: Bytes::copy_from_slice(pdu_data),
        };

        // If sending fails or reaches the deadline, remove the registration so
        // the slot is not retained until a later scheduler pass.
        match poll_before_deadline(attempt_deadline, sink.send(frame)).await {
            Some(Ok(())) => {}
            Some(Err(e)) => {
                warn!(txn_id = txn_id.0, error = %e, "failed to send Modbus request");
                self.txn_mgr.remove(txn_id);
                return Err(ClientError::Transport(e));
            }
            None => {
                warn!(
                    txn_id = txn_id.0,
                    "Modbus request send reached its deadline"
                );
                self.txn_mgr.remove(txn_id);
                return Err(ClientError::Timeout);
            }
        }
        drop(sink);
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

        let got = response.function_code();

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

        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| ClientError::ShuttingDown)?;

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

    fn classify_attempt(
        &self,
        request_kind: RequestKind,
        attempt: u32,
        result: Result<OwnedResponsePdu, ClientError>,
    ) -> AttemptDisposition {
        match result {
            Ok(OwnedResponsePdu::Exception(exc))
                if exc.exception_code == ExceptionCode::Acknowledge =>
            {
                warn!(
                    attempt,
                    retry_suppression = "acknowledge_is_terminal",
                    "returning terminal Modbus Acknowledge exception"
                );
                AttemptDisposition::Complete(Ok(OwnedResponsePdu::Exception(exc)))
            }
            Ok(OwnedResponsePdu::Exception(exc))
                if exc.exception_code == ExceptionCode::ServerDeviceBusy
                    && self.config.retry.is_retryable(exc.exception_code) =>
            {
                AttemptDisposition::Retry {
                    reason: "server_device_busy",
                    error: ClientError::Exception(exc),
                }
            }
            Ok(response) => AttemptDisposition::Complete(Ok(response)),
            Err(ClientError::Timeout) if request_kind == RequestKind::ReplaySafe => {
                AttemptDisposition::Retry {
                    reason: "attempt_timeout",
                    error: ClientError::Timeout,
                }
            }
            Err(ClientError::Transport(TransportError::Timeout))
                if request_kind == RequestKind::ReplaySafe =>
            {
                AttemptDisposition::Retry {
                    reason: "transport_timeout",
                    error: ClientError::Transport(TransportError::Timeout),
                }
            }
            Err(error @ ClientError::Timeout) => {
                warn!(
                    attempt,
                    request_kind = request_kind.as_str(),
                    retry_suppression = "ambiguous_mutation",
                    "not retrying timed-out Modbus mutation"
                );
                AttemptDisposition::Complete(Err(error))
            }
            Err(error @ ClientError::Transport(TransportError::Timeout)) => {
                warn!(
                    attempt,
                    request_kind = request_kind.as_str(),
                    retry_suppression = "ambiguous_mutation",
                    "not retrying transport timeout for Modbus mutation"
                );
                AttemptDisposition::Complete(Err(error))
            }
            Err(error) => AttemptDisposition::Complete(Err(error)),
        }
    }

    /// Send a request with retry logic.
    #[tracing::instrument(
        level = "debug",
        skip(self, pdu_data),
        fields(
            unit_id = unit_id.0,
            function_code = function_code.code(),
            request_kind = request_kind.as_str(),
            max_retries = self.config.retry.max_retries
        )
    )]
    pub(crate) async fn send_with_retry(
        &self,
        unit_id: UnitId,
        function_code: FunctionCode,
        pdu_data: &[u8],
        request_kind: RequestKind,
    ) -> Result<OwnedResponsePdu, ClientError> {
        if !self.is_connected() {
            warn!("request rejected because client is disconnected");
            return Err(ClientError::NotConnected);
        }
        let pdu_len = checked_pdu_length(pdu_data.len())?;

        // Admit one logical operation and retain the permit across all attempts
        // and backoff sleeps.
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| ClientError::ShuttingDown)?;

        let operation_start = time::Instant::now();
        let budget = operation_budget(
            self.config.timeout,
            self.config.retry.retry_delay,
            self.config.retry.max_retries,
        );
        let operation_deadline = saturating_instant_add(operation_start, budget);
        debug!(
            request_kind = request_kind.as_str(),
            operation_budget = ?budget,
            operation_deadline = ?operation_deadline,
            "admitted Modbus operation"
        );

        let mut attempts_made = 0u32;
        let mut last_error = None;
        loop {
            if time::Instant::now() >= operation_deadline {
                return Err(ClientError::RetriesExhausted {
                    attempts: attempts_made,
                    last_error: Box::new(last_error.unwrap_or(ClientError::Timeout)),
                });
            }

            let attempt_start = time::Instant::now();
            let attempt_deadline =
                saturating_instant_add(attempt_start, self.config.timeout).min(operation_deadline);
            attempts_made = attempts_made.saturating_add(1);
            debug!(
                attempt = attempts_made,
                request_kind = request_kind.as_str(),
                attempt_deadline = ?attempt_deadline,
                "starting Modbus request attempt"
            );

            let result = self
                .send_request(unit_id, function_code, pdu_data, pdu_len, attempt_deadline)
                .await;

            let (retry_reason, error) =
                match self.classify_attempt(request_kind, attempts_made, result) {
                    AttemptDisposition::Complete(result) => return result,
                    AttemptDisposition::Retry { reason, error } => (reason, error),
                };

            warn!(
                attempt = attempts_made,
                request_kind = request_kind.as_str(),
                retry_reason,
                "Modbus request attempt is retryable"
            );
            last_error = Some(error);

            if attempts_made > self.config.retry.max_retries || attempts_made == u32::MAX {
                return Err(ClientError::RetriesExhausted {
                    attempts: attempts_made,
                    last_error: Box::new(last_error.expect("retryable attempt recorded an error")),
                });
            }

            let backoff_deadline =
                saturating_instant_add(time::Instant::now(), self.config.retry.retry_delay)
                    .min(operation_deadline);
            debug!(
                attempt = attempts_made,
                retry_reason,
                backoff_deadline = ?backoff_deadline,
                "waiting before Modbus request retry"
            );
            time::sleep_until(backoff_deadline).await;
        }
    }
}

impl<S: TransportSink + Send + 'static> Drop for ModbusClient<S> {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
        if let Some(h) = self.reader_handle.take() {
            h.abort();
        }
        if let Some(h) = self.deadline_handle.take() {
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

    #[test]
    fn operation_budget_and_deadline_arithmetic_saturate_without_panicking() {
        assert_eq!(
            operation_budget(Duration::MAX, Duration::MAX, u32::MAX),
            Duration::MAX
        );

        let start = time::Instant::now();
        assert!(saturating_instant_add(start, Duration::MAX) >= start);
    }
}
