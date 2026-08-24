//! `ModbusServer` TCP admission and sequential request execution.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_tcp::config::TcpServerConfig;
use rusty_modbus_tcp::listener::TcpServerListener;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_types::{ExceptionCode, MAX_PDU_SIZE, MbapHeader, UnitId};
use tokio::sync::watch;
use tokio::task::{JoinError, JoinSet};
use tokio::time::Instant;
use tracing::{debug, info, trace, warn};

use crate::config::{DeviceIdentification, ServerConfig};
use crate::error::ServerError;
use crate::handler;
use crate::lifecycle::{ServerLifecycle, ServerMetrics, ShutdownOutcome};
use crate::store::DataStore;

const ACCEPT_BACKOFF_INITIAL: Duration = Duration::from_millis(10);
const ACCEPT_BACKOFF_MAXIMUM: Duration = Duration::from_secs(1);

/// Async Modbus server, generic over the data store implementation.
///
/// [`Self::stop`] closes listener admission, drains admitted sequential
/// requests, and joins connection tasks. Dropping the server instead aborts its
/// supervisor without waiting; `Drop` does not guarantee graceful completion or
/// immediate rebinding of the listen address.
pub struct ModbusServer<S: DataStore> {
    config: ServerConfig,
    store: Arc<S>,
    local_addr: SocketAddr,
    lifecycle: Arc<ServerLifecycle>,
}

impl<S: DataStore + 'static> ModbusServer<S> {
    /// Validate configuration, bind the listen address, and start the supervisor.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::InvalidConfig`] before bind when a required limit
    /// is zero. Returns [`ServerError::Bind`] if the validated address cannot be
    /// bound.
    #[tracing::instrument(level = "debug", skip(config, store), fields(addr = %config.listen_addr, unit_id = config.unit_id.0))]
    pub async fn start(config: ServerConfig, store: Arc<S>) -> Result<Self, ServerError> {
        config.validate()?;

        let tcp_config = TcpServerConfig {
            max_connections: config.max_connections,
            ..config.tcp_config.clone()
        };

        let listener = TcpServerListener::bind(config.listen_addr, tcp_config)
            .await
            .map_err(|error| match error {
                rusty_modbus_tcp::TransportError::Io(io) => ServerError::Bind(io),
                other => ServerError::Transport(other),
            })?;

        let local_addr = listener.local_addr().map_err(|error| match error {
            rusty_modbus_tcp::TransportError::Io(io) => ServerError::Bind(io),
            other => ServerError::Transport(other),
        })?;
        info!(addr = %local_addr, unit_id = config.unit_id.0, "Modbus server listening");

        let lifecycle = ServerLifecycle::new(listener.metrics());
        let supervisor = tokio::spawn(supervise(
            listener,
            config.unit_id,
            Arc::clone(&store),
            config.device_id.clone(),
            Arc::clone(&lifecycle),
            lifecycle.shutdown_receiver(),
        ));
        lifecycle.install_supervisor(supervisor);

        Ok(Self {
            config,
            store,
            local_addr,
            lifecycle,
        })
    }

    /// Stop admission and wait for the stable shutdown outcome.
    ///
    /// The first call records one absolute deadline. Concurrent and later calls
    /// wait for or return the same outcome. Cancelling a caller does not cancel
    /// the shutdown coordinator. At the deadline, Tokio aborts unfinished tasks
    /// and the supervisor joins them before returning [`ShutdownOutcome::Forced`].
    /// Tokio task abort is cooperative, so a future that does not yield can delay
    /// completion beyond the deadline.
    pub async fn stop(&self) -> ShutdownOutcome {
        info!(addr = %self.local_addr, "stopping Modbus server");
        self.lifecycle.shutdown(self.config.shutdown_timeout).await
    }

    /// Collect the current server counters.
    #[must_use]
    pub fn metrics(&self) -> ServerMetrics {
        self.lifecycle.metrics()
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
        self.lifecycle.abort_owned_tasks();
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

async fn supervise<S: DataStore + 'static>(
    listener: TcpServerListener,
    unit_id: UnitId,
    store: Arc<S>,
    device_id: DeviceIdentification,
    lifecycle: Arc<ServerLifecycle>,
    mut shutdown_rx: watch::Receiver<Option<Instant>>,
) -> ShutdownOutcome {
    let mut connections = JoinSet::new();
    let mut backoff = AcceptBackoff::new();

    let deadline = loop {
        if let Some(deadline) = lifecycle.shutdown_deadline() {
            break deadline;
        }

        tokio::select! {
            biased;
            changed = shutdown_rx.changed() => {
                if changed.is_err() {
                    warn!("Modbus server shutdown channel closed");
                }
            }
            joined = connections.join_next(), if !connections.is_empty() => {
                if let Some(result) = joined {
                    report_connection_result(result);
                }
            }
            result = listener.accept() => {
                match result {
                    Ok((sink, stream, peer_addr, guard)) => {
                        backoff.reset();
                        // This state read shares the seal lock, so an accepted
                        // socket is either admitted before the seal or dropped.
                        if lifecycle.shutdown_deadline().is_some() {
                            drop((sink, stream, guard));
                            continue;
                        }
                        debug!(peer_addr = %peer_addr, "accepted Modbus server connection");
                        let connection_store = Arc::clone(&store);
                        let connection_device_id = device_id.clone();
                        let connection_lifecycle = Arc::clone(&lifecycle);
                        connections.spawn(async move {
                            let _guard = guard;
                            handle_connection(
                                sink,
                                stream,
                                peer_addr,
                                unit_id,
                                connection_store,
                                connection_device_id,
                                connection_lifecycle,
                            )
                            .await;
                        });
                    }
                    Err(error) => {
                        lifecycle.metrics_handle().record_accept_error();
                        let delay = backoff.failure_delay();
                        warn!(error = %error, ?delay, "Modbus server accept failed");
                        let _ = backoff_interrupted(delay, &mut shutdown_rx).await;
                    }
                }
            }
        }
    };

    // Closing the listen socket precedes every connection-drain wait.
    drop(listener);
    drain_connections(&mut connections, deadline).await
}

async fn drain_connections(connections: &mut JoinSet<()>, deadline: Instant) -> ShutdownOutcome {
    while !connections.is_empty() {
        tokio::select! {
            biased;
            () = tokio::time::sleep_until(deadline) => {
                warn!(remaining = connections.len(), "Modbus server shutdown deadline elapsed");
                connections.abort_all();
                while let Some(result) = connections.join_next().await {
                    report_connection_result(result);
                }
                return ShutdownOutcome::Forced;
            }
            joined = connections.join_next() => {
                if let Some(result) = joined {
                    report_connection_result(result);
                }
            }
        }
    }
    ShutdownOutcome::Drained
}

fn report_connection_result(result: Result<(), JoinError>) {
    if let Err(error) = result
        && !error.is_cancelled()
    {
        warn!(%error, "Modbus server connection task failed");
    }
}

async fn handle_connection<S: DataStore>(
    mut sink: rusty_modbus_tcp::TcpSink,
    mut stream: rusty_modbus_tcp::TcpRecvStream,
    peer_addr: SocketAddr,
    unit_id: UnitId,
    store: Arc<S>,
    device_id: DeviceIdentification,
    lifecycle: Arc<ServerLifecycle>,
) {
    let mut shutdown_rx = lifecycle.shutdown_receiver();
    loop {
        if lifecycle.shutdown_deadline().is_some() {
            break;
        }

        let frame = tokio::select! {
            biased;
            _ = shutdown_rx.changed() => None,
            result = stream.recv() => match result {
                Ok(frame) => Some(frame),
                Err(error) => {
                    trace!(peer_addr = %peer_addr, error = %error, "Modbus server receive ended");
                    None
                }
            },
        };
        let Some(frame) = frame else {
            break;
        };

        // Rechecking under the lifecycle lock closes the recv/watch race.
        let Some(_request_guard) = lifecycle.admit_request() else {
            break;
        };

        let request_unit_id = UnitId(frame.unit_id());
        let pdu_len = frame.pdu.len();
        trace!(
            peer_addr = %peer_addr,
            request_unit_id = request_unit_id.0,
            pdu_len,
            "received Modbus server request"
        );

        if request_unit_id.0 != unit_id.0
            && !request_unit_id.is_broadcast()
            && !request_unit_id.is_tcp_device()
        {
            debug!(
                peer_addr = %peer_addr,
                request_unit_id = request_unit_id.0,
                server_unit_id = unit_id.0,
                "discarding request for different unit id"
            );
            continue;
        }

        let txn_id = match frame.header {
            FrameHeader::Mbap(header) => header.transaction_id.get(),
            FrameHeader::Rtu { .. } => 0,
        };

        if let Some(response_pdu) =
            handler::process_request(&frame.pdu, request_unit_id, store.as_ref(), &device_id).await
        {
            let Some(response_frame) = response_frame(txn_id, request_unit_id, response_pdu) else {
                warn!(peer_addr = %peer_addr, txn_id, "dropping empty Modbus response PDU");
                break;
            };
            if let Err(error) = sink.send(response_frame).await {
                debug!(peer_addr = %peer_addr, txn_id, error = %error, "failed to send Modbus response");
                break;
            }
            trace!(peer_addr = %peer_addr, txn_id, "sent Modbus server response");
        }
    }
    debug!(peer_addr = %peer_addr, "Modbus server connection closed");
}

#[derive(Debug)]
struct AcceptBackoff {
    next: Duration,
}

impl AcceptBackoff {
    fn new() -> Self {
        Self {
            next: ACCEPT_BACKOFF_INITIAL,
        }
    }

    fn failure_delay(&mut self) -> Duration {
        let delay = self.next;
        self.next = self.next.saturating_mul(2).min(ACCEPT_BACKOFF_MAXIMUM);
        delay
    }

    fn reset(&mut self) {
        self.next = ACCEPT_BACKOFF_INITIAL;
    }
}

async fn backoff_interrupted(
    delay: Duration,
    shutdown_rx: &mut watch::Receiver<Option<Instant>>,
) -> bool {
    if shutdown_rx.borrow().is_some() {
        return true;
    }
    tokio::select! {
        biased;
        changed = shutdown_rx.changed() => changed.is_err() || shutdown_rx.borrow().is_some(),
        () = tokio::time::sleep(delay) => false,
    }
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
    let function_code = response_pdu.first().copied()?;
    if response_pdu.len() <= MAX_PDU_SIZE {
        return Some(response_pdu);
    }

    warn!(
        function_code,
        pdu_len = response_pdu.len(),
        max_pdu_size = MAX_PDU_SIZE,
        "server response exceeded Modbus PDU limit"
    );
    Some(vec![
        function_code | 0x80,
        ExceptionCode::ServerDeviceFailure.code(),
    ])
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

    #[test]
    fn accept_backoff_caps_and_resets() {
        let mut backoff = AcceptBackoff::new();
        let delays: Vec<_> = (0..9).map(|_| backoff.failure_delay()).collect();
        assert_eq!(
            delays,
            [10, 20, 40, 80, 160, 320, 640, 1_000, 1_000].map(Duration::from_millis)
        );

        backoff.reset();
        assert_eq!(backoff.failure_delay(), ACCEPT_BACKOFF_INITIAL);
    }

    #[tokio::test]
    async fn accept_backoff_is_interrupted_by_shutdown() {
        let (shutdown_tx, mut shutdown_rx) = watch::channel(None);
        let wait = tokio::spawn(async move {
            backoff_interrupted(Duration::from_mins(1), &mut shutdown_rx).await
        });
        tokio::task::yield_now().await;

        shutdown_tx.send_replace(Some(Instant::now()));

        assert!(wait.await.unwrap());
    }

    #[tokio::test]
    async fn drain_deadline_wins_over_ready_last_connection() {
        let mut connections = JoinSet::new();
        let (release, released) = tokio::sync::oneshot::channel();
        let completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let task_completed = Arc::clone(&completed);
        connections.spawn(async move {
            released.await.unwrap();
            task_completed.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        tokio::task::yield_now().await;

        let deadline = Instant::now();
        tokio::time::sleep(Duration::from_millis(1)).await;
        release.send(()).unwrap();
        while !completed.load(std::sync::atomic::Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }

        assert_eq!(
            drain_connections(&mut connections, deadline).await,
            ShutdownOutcome::Forced
        );
        assert!(connections.is_empty());
    }
}
