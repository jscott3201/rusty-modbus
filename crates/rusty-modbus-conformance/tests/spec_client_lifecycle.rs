//! Project lifecycle policy: client shutdown seals admission before bounded drain.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus_client::{ClientConfig, ClientError, ModbusClient, RetryConfig};
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_tcp::TransportError;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_types::{MbapHeader, UnitId};
use tokio::sync::mpsc;

struct LifecycleSink {
    sent: mpsc::UnboundedSender<Frame>,
    completions: mpsc::UnboundedReceiver<()>,
}

impl TransportSink for LifecycleSink {
    async fn send(&mut self, frame: Frame) -> Result<(), TransportError> {
        self.sent
            .send(frame)
            .map_err(|_| TransportError::Disconnected)?;
        self.completions
            .recv()
            .await
            .ok_or(TransportError::Disconnected)
    }
}

struct LifecycleStream {
    responses: mpsc::UnboundedReceiver<Frame>,
    dropped: Arc<AtomicBool>,
}

impl TransportStream for LifecycleStream {
    async fn recv(&mut self) -> Result<Frame, TransportError> {
        self.responses
            .recv()
            .await
            .ok_or(TransportError::Disconnected)
    }
}

impl Drop for LifecycleStream {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

struct Controls {
    sent: mpsc::UnboundedReceiver<Frame>,
    completions: mpsc::UnboundedSender<()>,
    responses: mpsc::UnboundedSender<Frame>,
    stream_dropped: Arc<AtomicBool>,
}

fn transport() -> (LifecycleSink, LifecycleStream, Controls) {
    let (sent_tx, sent_rx) = mpsc::unbounded_channel();
    let (completion_tx, completion_rx) = mpsc::unbounded_channel();
    let (response_tx, response_rx) = mpsc::unbounded_channel();
    let stream_dropped = Arc::new(AtomicBool::new(false));
    (
        LifecycleSink {
            sent: sent_tx,
            completions: completion_rx,
        },
        LifecycleStream {
            responses: response_rx,
            dropped: Arc::clone(&stream_dropped),
        },
        Controls {
            sent: sent_rx,
            completions: completion_tx,
            responses: response_tx,
            stream_dropped,
        },
    )
}

fn config() -> ClientConfig {
    ClientConfig {
        timeout: Duration::from_secs(1),
        shutdown_timeout: Duration::from_millis(100),
        max_in_flight: 1,
        retry: RetryConfig {
            max_retries: 0,
            ..RetryConfig::default()
        },
        ..ClientConfig::default()
    }
}

fn response(request: &Frame) -> Frame {
    let FrameHeader::Mbap(header) = request.header else {
        panic!("expected MBAP request");
    };
    Frame {
        header: FrameHeader::Mbap(MbapHeader::new(
            header.transaction_id.get(),
            request.unit_id(),
            4,
        )),
        pdu: Bytes::from_static(&[0x03, 0x02, 0x00, 0x2A]),
    }
}

#[tokio::test]
async fn graceful_shutdown_seals_admission_but_keeps_response_processing_alive() {
    let (sink, stream, mut controls) = transport();
    let client = Arc::new(ModbusClient::from_transport(sink, stream, config()));

    controls.completions.send(()).unwrap();
    let request_client = Arc::clone(&client);
    let request =
        tokio::spawn(async move { request_client.read_holding_registers(UnitId(1), 0, 1).await });
    let sent = controls.sent.recv().await.unwrap();

    let shutdown_client = Arc::clone(&client);
    let shutdown = tokio::spawn(async move { shutdown_client.shutdown().await });
    tokio::task::yield_now().await;
    assert!(!shutdown.is_finished());

    assert!(matches!(
        client.read_holding_registers(UnitId(1), 0, 1).await,
        Err(ClientError::NotConnected)
    ));
    assert!(controls.sent.try_recv().is_err());

    controls.responses.send(response(&sent)).unwrap();
    assert_eq!(request.await.unwrap().unwrap(), vec![0x002A]);
    shutdown.await.unwrap();
    assert!(controls.stream_dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn abort_cancels_broadcast_and_shutdown_can_finalize_tasks() {
    let (sink, stream, mut controls) = transport();
    let client = Arc::new(ModbusClient::from_transport(sink, stream, config()));

    let broadcast_client = Arc::clone(&client);
    let broadcast = tokio::spawn(async move {
        broadcast_client
            .write_single_register(UnitId(0), 0, 1)
            .await
    });
    assert_eq!(controls.sent.recv().await.unwrap().unit_id(), 0);

    client.abort();
    assert!(matches!(
        broadcast.await.unwrap(),
        Err(ClientError::ShuttingDown)
    ));
    assert!(matches!(
        client.read_holding_registers(UnitId(1), 0, 1).await,
        Err(ClientError::NotConnected)
    ));
    client.shutdown().await;
    assert!(controls.stream_dropped.load(Ordering::SeqCst));
}
