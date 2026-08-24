//! Integration tests for ModbusClient.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus_client::{ClientConfig, ClientError, ModbusClient, RetryConfig};
use rusty_modbus_codec::EncodeError;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_tcp::TransportError;
use rusty_modbus_tcp::config::TcpServerConfig;
use rusty_modbus_tcp::listener::TcpServerListener;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_types::{ExceptionCode, MbapHeader, UnitId};
use tokio::sync::mpsc;

enum SendOutcome {
    Success,
    Failure,
    Timeout,
}

struct ControlledSink {
    sent_tx: mpsc::UnboundedSender<Frame>,
    outcome_rx: mpsc::UnboundedReceiver<SendOutcome>,
}

impl TransportSink for ControlledSink {
    async fn send(&mut self, frame: Frame) -> Result<(), TransportError> {
        self.sent_tx
            .send(frame)
            .map_err(|_| TransportError::Disconnected)?;
        match self.outcome_rx.recv().await {
            Some(SendOutcome::Success) => Ok(()),
            Some(SendOutcome::Failure) => Err(TransportError::Io(std::io::Error::other(
                "controlled send failure",
            ))),
            Some(SendOutcome::Timeout) => Err(TransportError::Timeout),
            None => Err(TransportError::Disconnected),
        }
    }
}

struct ControlledStream {
    response_rx: mpsc::UnboundedReceiver<Frame>,
    dropped: Arc<AtomicBool>,
}

impl TransportStream for ControlledStream {
    async fn recv(&mut self) -> Result<Frame, TransportError> {
        self.response_rx
            .recv()
            .await
            .ok_or(TransportError::Disconnected)
    }
}

impl Drop for ControlledStream {
    fn drop(&mut self) {
        self.dropped
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

struct TransportControls {
    sent_rx: mpsc::UnboundedReceiver<Frame>,
    outcome_tx: mpsc::UnboundedSender<SendOutcome>,
    response_tx: mpsc::UnboundedSender<Frame>,
    stream_dropped: Arc<AtomicBool>,
}

fn controlled_transport() -> (ControlledSink, ControlledStream, TransportControls) {
    let (sent_tx, sent_rx) = mpsc::unbounded_channel();
    let (outcome_tx, outcome_rx) = mpsc::unbounded_channel();
    let (response_tx, response_rx) = mpsc::unbounded_channel();
    let stream_dropped = Arc::new(AtomicBool::new(false));

    (
        ControlledSink {
            sent_tx,
            outcome_rx,
        },
        ControlledStream {
            response_rx,
            dropped: Arc::clone(&stream_dropped),
        },
        TransportControls {
            sent_rx,
            outcome_tx,
            response_tx,
            stream_dropped,
        },
    )
}

fn rtu_frame(unit_id: u8, pdu: impl Into<Bytes>) -> Frame {
    Frame {
        header: FrameHeader::Rtu { unit_id },
        pdu: pdu.into(),
    }
}

fn mbap_response(request: &Frame, pdu: impl Into<Bytes>) -> Frame {
    let txn_id = match request.header {
        FrameHeader::Mbap(header) => header.transaction_id.get(),
        FrameHeader::Rtu { .. } => panic!("expected MBAP request"),
    };
    let pdu = pdu.into();
    Frame {
        header: FrameHeader::Mbap(MbapHeader::new(txn_id, request.unit_id(), pdu.len() as u16)),
        pdu,
    }
}

async fn poll_once<F: Future>(future: Pin<&mut F>) -> Option<F::Output> {
    tokio::select! {
        biased;
        output = future => Some(output),
        () = std::future::ready(()) => None,
    }
}

/// Start a test server that responds to ReadHoldingRegisters with [0x1234, 0x5678].
async fn start_register_server() -> SocketAddr {
    let listener =
        TcpServerListener::bind("127.0.0.1:0".parse().unwrap(), TcpServerConfig::default())
            .await
            .unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        while let Ok((mut sink, mut stream, _, _guard)) = listener.accept().await {
            tokio::spawn(async move {
                while let Ok(req_frame) = stream.recv().await {
                    let txn_id = match req_frame.header {
                        FrameHeader::Mbap(h) => h.transaction_id.get(),
                        FrameHeader::Rtu { .. } => 0,
                    };
                    let unit_id = req_frame.unit_id();
                    let fc = req_frame.pdu[0];

                    let resp_pdu: Vec<u8> = if fc == 0x03 || fc == 0x04 {
                        // ReadHoldingRegisters/ReadInputRegisters response.
                        vec![fc, 0x04, 0x12, 0x34, 0x56, 0x78]
                    } else if fc == 0x06 {
                        // WriteSingleRegister echo.
                        req_frame.pdu.to_vec()
                    } else if fc == 0x10 {
                        // WriteMultipleRegisters response: echo first 4 bytes (FC, addr, qty).
                        let mut resp = vec![fc];
                        resp.extend_from_slice(&req_frame.pdu[1..5]);
                        resp
                    } else if fc == 0x05 {
                        // WriteSingleCoil echo.
                        req_frame.pdu.to_vec()
                    } else {
                        // Unknown FC — return exception.
                        vec![fc | 0x80, 0x01]
                    };

                    let header = MbapHeader::new(txn_id, unit_id, resp_pdu.len() as u16);
                    let resp_frame = Frame {
                        header: FrameHeader::Mbap(header),
                        pdu: Bytes::from(resp_pdu),
                    };
                    if sink.send(resp_frame).await.is_err() {
                        break;
                    }
                }
            });
        }
    });

    addr
}

/// Start a server that returns exception on first request, then succeeds.
async fn start_busy_then_ok_server() -> SocketAddr {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    let listener =
        TcpServerListener::bind("127.0.0.1:0".parse().unwrap(), TcpServerConfig::default())
            .await
            .unwrap();
    let addr = listener.local_addr().unwrap();
    let call_count = Arc::new(AtomicU32::new(0));

    tokio::spawn(async move {
        while let Ok((mut sink, mut stream, _, _guard)) = listener.accept().await {
            let count = Arc::clone(&call_count);
            tokio::spawn(async move {
                while let Ok(req_frame) = stream.recv().await {
                    let txn_id = match req_frame.header {
                        FrameHeader::Mbap(h) => h.transaction_id.get(),
                        FrameHeader::Rtu { .. } => 0,
                    };
                    let n = count.fetch_add(1, Ordering::Relaxed);

                    let resp_pdu = if n == 0 {
                        // First call: ServerDeviceBusy exception.
                        vec![0x83, 0x06]
                    } else {
                        // Subsequent: success.
                        vec![0x03, 0x02, 0x00, 0x42]
                    };

                    let header =
                        MbapHeader::new(txn_id, req_frame.unit_id(), resp_pdu.len() as u16);
                    let resp = Frame {
                        header: FrameHeader::Mbap(header),
                        pdu: Bytes::from(resp_pdu),
                    };
                    if sink.send(resp).await.is_err() {
                        break;
                    }
                }
            });
        }
    });

    addr
}

fn default_config() -> ClientConfig {
    ClientConfig {
        timeout: Duration::from_secs(2),
        ..ClientConfig::default()
    }
}

fn assert_quantity_encode_error<T>(result: Result<T, ClientError>, quantity: u16) {
    assert!(
        matches!(
            result,
            Err(ClientError::Encode(EncodeError::QuantityOutOfRange { quantity: got }))
                if got == quantity
        ),
        "expected quantity encode error for {quantity}"
    );
}

fn assert_file_byte_count_encode_error<T>(
    result: Result<T, ClientError>,
    count: usize,
    minimum: usize,
    maximum: usize,
) {
    assert!(
        matches!(
            result,
            Err(ClientError::Encode(EncodeError::ByteCountOutOfRange {
                count: got,
                minimum: got_minimum,
                maximum: got_maximum,
            })) if got == count && got_minimum == minimum && got_maximum == maximum
        ),
        "expected file byte-count encode error for {count}"
    );
}

fn assert_echo_mismatch<T>(
    result: Result<T, ClientError>,
    field: &'static str,
    expected: u16,
    got: u16,
) {
    assert!(
        matches!(
            result,
            Err(ClientError::UnexpectedResponseEcho {
                field: got_field,
                expected: got_expected,
                got: got_value,
            }) if got_field == field && got_expected == expected && got_value == got
        ),
        "expected echo mismatch for {field}: {expected:#06x} != {got:#06x}"
    );
}

fn assert_unexpected_length<T>(
    result: Result<T, ClientError>,
    function_code: u8,
    expected: usize,
    actual: usize,
) {
    assert!(
        matches!(
            result,
            Err(ClientError::UnexpectedResponseLength {
                function_code: got_function_code,
                expected: got_expected,
                actual: got_actual,
            }) if got_function_code == function_code
                && got_expected == expected
                && got_actual == actual
        ),
        "expected response length mismatch for function {function_code:#04x}"
    );
}

fn assert_unexpected_padding<T>(
    result: Result<T, ClientError>,
    function_code: u8,
    invalid_mask: u8,
    actual: u8,
) {
    assert!(
        matches!(
            result,
            Err(ClientError::UnexpectedResponsePadding {
                function_code: got_function_code,
                invalid_mask: got_invalid_mask,
                actual: got_actual,
            }) if got_function_code == function_code
                && got_invalid_mask == invalid_mask
                && got_actual == actual
        ),
        "expected response padding mismatch for function {function_code:#04x}"
    );
}

fn assert_no_sent_frame(controls: &mut TransportControls) {
    assert!(matches!(
        controls.sent_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

async fn assert_mutating_timeout_once<F, T>(
    controls: &mut TransportControls,
    future: F,
    expected_function: u8,
) where
    F: Future<Output = Result<T, ClientError>>,
{
    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut future = Box::pin(future);
    assert!(poll_once(future.as_mut()).await.is_none());
    assert_eq!(
        controls.sent_rx.try_recv().unwrap().pdu[0],
        expected_function
    );

    tokio::time::advance(Duration::from_millis(20)).await;
    assert!(matches!(
        poll_once(future.as_mut()).await,
        Some(Err(ClientError::Timeout))
    ));
    assert_no_sent_frame(controls);
}

#[tokio::test]
async fn read_holding_registers() {
    let addr = start_register_server().await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    let regs = client
        .read_holding_registers(UnitId(0xFF), 0, 2)
        .await
        .unwrap();

    assert_eq!(regs, vec![0x1234, 0x5678]);
}

#[tokio::test]
async fn read_input_registers() {
    let addr = start_register_server().await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    let regs = client
        .read_input_registers(UnitId(0xFF), 0, 2)
        .await
        .unwrap();

    assert_eq!(regs, vec![0x1234, 0x5678]);
}

#[tokio::test]
async fn write_single_register() {
    let addr = start_register_server().await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    client
        .write_single_register(UnitId(0xFF), 0x0001, 0xABCD)
        .await
        .unwrap();
}

#[tokio::test]
async fn write_multiple_registers() {
    let addr = start_register_server().await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    client
        .write_multiple_registers(UnitId(0xFF), 0x0001, &[0x0001, 0x0002])
        .await
        .unwrap();
}

#[tokio::test]
async fn write_single_coil() {
    let addr = start_register_server().await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    client
        .write_single_coil(UnitId(0xFF), 0x0000, true)
        .await
        .unwrap();
}

#[tokio::test]
async fn broadcast_read_rejected() {
    let addr = start_register_server().await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    let result = client.read_holding_registers(UnitId(0x00), 0, 1).await;
    assert!(matches!(result, Err(ClientError::BroadcastReadNotAllowed)));
}

#[tokio::test]
async fn rtu_broadcast_waits_for_pending_unicast() {
    let (sink, stream, mut controls) = controlled_transport();
    let client = ModbusClient::from_rtu_transport(sink, stream, default_config());

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut unicast = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(unicast.as_mut()).await.is_none());
    let sent_unicast = controls.sent_rx.try_recv().unwrap();
    assert_eq!(sent_unicast.unit_id(), 1);
    assert_eq!(sent_unicast.pdu[0], 0x03);

    let mut broadcast = Box::pin(client.write_single_register(UnitId(0), 7, 0x1234));
    assert!(poll_once(broadcast.as_mut()).await.is_none());
    assert!(matches!(
        controls.sent_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));

    controls
        .response_tx
        .send(rtu_frame(1, vec![0x03, 0x02, 0x00, 0x2A]))
        .unwrap();
    assert_eq!(unicast.await.unwrap(), vec![0x002A]);

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    broadcast.await.unwrap();
    let sent_broadcast = controls.sent_rx.try_recv().unwrap();
    assert_eq!(sent_broadcast.unit_id(), 0);
    assert_eq!(sent_broadcast.pdu[0], 0x06);
    assert!(matches!(
        controls.sent_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn repeated_rtu_broadcasts_send_once_without_response_or_transaction() {
    let (sink, stream, mut controls) = controlled_transport();
    let client = ModbusClient::from_rtu_transport(sink, stream, default_config());

    for value in 0..20 {
        controls.outcome_tx.send(SendOutcome::Success).unwrap();
        client
            .write_single_register(UnitId(0), value, value)
            .await
            .unwrap();
        let frame = controls.sent_rx.try_recv().unwrap();
        assert_eq!(frame.unit_id(), 0);
        assert_eq!(frame.pdu[0], 0x06);
        assert!(matches!(
            controls.sent_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut unicast = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(unicast.as_mut()).await.is_none());
    assert_eq!(controls.sent_rx.try_recv().unwrap().unit_id(), 1);
    controls
        .response_tx
        .send(rtu_frame(1, vec![0x03, 0x02, 0x00, 0x2A]))
        .unwrap();
    assert_eq!(unicast.await.unwrap(), vec![0x002A]);
}

#[tokio::test]
async fn rtu_broadcast_send_failure_releases_admission() {
    let (sink, stream, mut controls) = controlled_transport();
    let client = ModbusClient::from_rtu_transport(sink, stream, default_config());

    controls.outcome_tx.send(SendOutcome::Failure).unwrap();
    let result = client.write_single_register(UnitId(0), 1, 1).await;
    assert!(matches!(result, Err(ClientError::Transport(_))));
    assert_eq!(controls.sent_rx.try_recv().unwrap().unit_id(), 0);

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    client.write_single_register(UnitId(0), 2, 2).await.unwrap();
    assert_eq!(controls.sent_rx.try_recv().unwrap().unit_id(), 0);
    assert!(matches!(
        controls.sent_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn cancelled_waiting_rtu_broadcast_does_not_send_or_leak_admission() {
    let (sink, stream, mut controls) = controlled_transport();
    let client = ModbusClient::from_rtu_transport(sink, stream, default_config());

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut unicast = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(unicast.as_mut()).await.is_none());
    assert_eq!(controls.sent_rx.try_recv().unwrap().unit_id(), 1);

    let mut cancelled = Box::pin(client.write_single_register(UnitId(0), 1, 1));
    assert!(poll_once(cancelled.as_mut()).await.is_none());
    drop(cancelled);
    assert!(matches!(
        controls.sent_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));

    controls
        .response_tx
        .send(rtu_frame(1, vec![0x03, 0x02, 0x00, 0x2A]))
        .unwrap();
    assert_eq!(unicast.await.unwrap(), vec![0x002A]);

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    client.write_single_register(UnitId(0), 2, 2).await.unwrap();
    assert_eq!(controls.sent_rx.try_recv().unwrap().unit_id(), 0);
    assert!(matches!(
        controls.sent_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn cancelled_acquired_rtu_broadcast_releases_admission() {
    let (sink, stream, mut controls) = controlled_transport();
    let client = ModbusClient::from_rtu_transport(sink, stream, default_config());

    let mut cancelled = Box::pin(client.write_single_register(UnitId(0), 1, 1));
    assert!(poll_once(cancelled.as_mut()).await.is_none());
    assert_eq!(controls.sent_rx.try_recv().unwrap().unit_id(), 0);
    drop(cancelled);

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    client.write_single_register(UnitId(0), 2, 2).await.unwrap();
    assert_eq!(controls.sent_rx.try_recv().unwrap().unit_id(), 0);
    assert!(matches!(
        controls.sent_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn tcp_broadcast_uses_one_admission_slot_without_global_serialization() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        max_in_flight: 2,
        ..default_config()
    };
    let client = ModbusClient::from_transport(sink, stream, config);

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut unicast = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(unicast.as_mut()).await.is_none());
    assert_eq!(controls.sent_rx.try_recv().unwrap().unit_id(), 1);

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    client.write_single_register(UnitId(0), 1, 1).await.unwrap();
    assert_eq!(controls.sent_rx.try_recv().unwrap().unit_id(), 0);

    controls
        .response_tx
        .send(Frame {
            header: FrameHeader::Mbap(MbapHeader::new(1, 1, 4)),
            pdu: Bytes::from_static(&[0x03, 0x02, 0x00, 0x2A]),
        })
        .unwrap();
    assert_eq!(unicast.await.unwrap(), vec![0x002A]);
}

#[tokio::test]
async fn tcp_response_with_wrong_unit_id_is_rejected() {
    let (sink, stream, mut controls) = controlled_transport();
    let client = ModbusClient::from_transport(sink, stream, default_config());

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut request = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(request.as_mut()).await.is_none());
    let sent = controls.sent_rx.try_recv().unwrap();
    let txn_id = match sent.header {
        FrameHeader::Mbap(header) => header.transaction_id.get(),
        FrameHeader::Rtu { .. } => panic!("expected MBAP request"),
    };

    controls
        .response_tx
        .send(Frame {
            header: FrameHeader::Mbap(MbapHeader::new(txn_id, 2, 4)),
            pdu: Bytes::from_static(&[0x03, 0x02, 0x00, 0x2A]),
        })
        .unwrap();

    let result = request.await;
    assert!(
        matches!(
            result,
            Err(ClientError::UnexpectedResponseUnitId {
                expected: 1,
                got: 2
            })
        ),
        "expected a typed unit ID mismatch, got {result:?}"
    );
}

#[tokio::test]
async fn rtu_response_with_wrong_unit_id_is_ignored() {
    let (sink, stream, mut controls) = controlled_transport();
    let client = ModbusClient::from_rtu_transport(sink, stream, default_config());

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut request = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(request.as_mut()).await.is_none());
    let sent = controls.sent_rx.try_recv().unwrap();
    assert_eq!(sent.unit_id(), 1);

    controls
        .response_tx
        .send(rtu_frame(2, vec![0x03, 0x02, 0x00, 0x11]))
        .unwrap();
    controls
        .response_tx
        .send(rtu_frame(1, vec![0x03, 0x02, 0x00, 0x2A]))
        .unwrap();

    assert_eq!(request.await.unwrap(), vec![0x002A]);
}

#[tokio::test]
async fn unicast_send_failure_reclaims_only_its_transaction() {
    let (sink, stream, mut controls) = controlled_transport();
    let client = ModbusClient::from_transport(sink, stream, default_config());

    controls.outcome_tx.send(SendOutcome::Failure).unwrap();
    let result = client.read_holding_registers(UnitId(1), 0, 1).await;
    assert!(matches!(result, Err(ClientError::Transport(_))));
    let failed_request = controls.sent_rx.try_recv().unwrap();
    assert_eq!(failed_request.unit_id(), 1);

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut request = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(request.as_mut()).await.is_none());
    let sent = controls.sent_rx.try_recv().unwrap();
    let txn_id = match sent.header {
        FrameHeader::Mbap(header) => header.transaction_id.get(),
        FrameHeader::Rtu { .. } => panic!("expected MBAP request"),
    };
    controls
        .response_tx
        .send(Frame {
            header: FrameHeader::Mbap(MbapHeader::new(txn_id, 1, 4)),
            pdu: Bytes::from_static(&[0x03, 0x02, 0x00, 0x2A]),
        })
        .unwrap();

    assert_eq!(request.await.unwrap(), vec![0x002A]);
}

#[tokio::test]
async fn unicast_send_failure_wins_over_early_response() {
    let (sink, stream, mut controls) = controlled_transport();
    let client = ModbusClient::from_transport(sink, stream, default_config());

    let mut request = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(request.as_mut()).await.is_none());
    let sent = controls.sent_rx.try_recv().unwrap();
    let txn_id = match sent.header {
        FrameHeader::Mbap(header) => header.transaction_id.get(),
        FrameHeader::Rtu { .. } => panic!("expected MBAP request"),
    };

    controls
        .response_tx
        .send(Frame {
            header: FrameHeader::Mbap(MbapHeader::new(txn_id, 1, 4)),
            pdu: Bytes::from_static(&[0x03, 0x02, 0x00, 0x2A]),
        })
        .unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    controls.outcome_tx.send(SendOutcome::Failure).unwrap();

    assert!(matches!(request.await, Err(ClientError::Transport(_))));
}

#[tokio::test]
async fn device_identification_broadcast_read_is_rejected_before_transport() {
    let (sink, stream, mut controls) = controlled_transport();
    let client = ModbusClient::from_rtu_transport(sink, stream, default_config());

    let mut request = Box::pin(client.read_device_identification(UnitId(0)));
    let result = poll_once(request.as_mut())
        .await
        .expect("broadcast read rejection must not wait for transport");
    assert!(matches!(result, Err(ClientError::BroadcastReadNotAllowed)));
    assert!(matches!(
        controls.sent_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn pipelining_concurrent_requests() {
    let addr = start_register_server().await;
    let client = std::sync::Arc::new(ModbusClient::connect(addr, default_config()).await.unwrap());

    let mut handles = Vec::new();
    for _ in 0..4 {
        let c = std::sync::Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            c.read_holding_registers(UnitId(0xFF), 0, 2).await
        }));
    }

    for h in handles {
        let result = h.await.unwrap().unwrap();
        assert_eq!(result, vec![0x1234, 0x5678]);
    }
}

#[tokio::test]
async fn retry_on_server_device_busy() {
    let addr = start_busy_then_ok_server().await;
    let config = ClientConfig {
        timeout: Duration::from_secs(2),
        retry: RetryConfig {
            max_retries: 3,
            retry_delay: Duration::from_millis(50),
            ..RetryConfig::default()
        },
        ..ClientConfig::default()
    };

    let client = ModbusClient::connect(addr, config).await.unwrap();
    let regs = client
        .read_holding_registers(UnitId(0xFF), 0, 1)
        .await
        .unwrap();

    assert_eq!(regs, vec![0x0042]);
}

#[tokio::test(start_paused = true)]
async fn retry_policy_mutating_response_timeout_sends_once() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        timeout: Duration::from_millis(20),
        retry: RetryConfig {
            max_retries: 1,
            retry_delay: Duration::from_millis(10),
            ..RetryConfig::default()
        },
        ..ClientConfig::default()
    };
    let client = ModbusClient::from_transport(sink, stream, config);

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut request = Box::pin(client.write_single_register(UnitId(1), 7, 0x1234));
    assert!(poll_once(request.as_mut()).await.is_none());
    let first = controls.sent_rx.try_recv().unwrap();
    assert_eq!(first.pdu[0], 0x06);

    tokio::time::advance(Duration::from_millis(20)).await;
    assert!(matches!(request.await, Err(ClientError::Timeout)));
    assert!(matches!(
        controls.sent_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test(start_paused = true)]
async fn retry_policy_acknowledge_is_terminal_even_when_configured() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        timeout: Duration::from_millis(100),
        retry: RetryConfig {
            max_retries: 1,
            retry_delay: Duration::from_millis(10),
            retryable_exceptions: vec![ExceptionCode::Acknowledge],
        },
        ..ClientConfig::default()
    };
    let client = ModbusClient::from_transport(sink, stream, config);

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut request = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(request.as_mut()).await.is_none());
    let first = controls.sent_rx.try_recv().unwrap();
    controls
        .response_tx
        .send(mbap_response(&first, Bytes::from_static(&[0x83, 0x05])))
        .unwrap();

    let mut result = poll_once(request.as_mut()).await;
    if result.is_none() {
        tokio::time::advance(Duration::from_millis(10)).await;
        result = poll_once(request.as_mut()).await;
    }

    assert!(matches!(
        result,
        Some(Err(ClientError::Exception(exc)))
            if exc.exception_code == ExceptionCode::Acknowledge
    ));
    assert!(matches!(
        controls.sent_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test(start_paused = true)]
async fn attempt_deadline_is_not_delayed_by_periodic_sweep() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        timeout: Duration::from_millis(20),
        retry: RetryConfig {
            max_retries: 0,
            ..RetryConfig::default()
        },
        ..ClientConfig::default()
    };
    let client = ModbusClient::from_transport(sink, stream, config);

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut request = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(request.as_mut()).await.is_none());
    assert_eq!(controls.sent_rx.try_recv().unwrap().pdu[0], 0x03);

    tokio::time::advance(Duration::from_millis(20)).await;
    assert!(matches!(
        poll_once(request.as_mut()).await,
        Some(Err(ClientError::RetriesExhausted {
            attempts: 1,
            last_error,
        })) if matches!(*last_error, ClientError::Timeout)
    ));
}

#[tokio::test(start_paused = true)]
async fn replay_safe_response_timeout_retries_then_succeeds() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        timeout: Duration::from_millis(20),
        retry: RetryConfig {
            max_retries: 1,
            retry_delay: Duration::from_millis(10),
            ..RetryConfig::default()
        },
        ..ClientConfig::default()
    };
    let client = ModbusClient::from_transport(sink, stream, config);

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut request = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(request.as_mut()).await.is_none());
    assert_eq!(controls.sent_rx.try_recv().unwrap().pdu[0], 0x03);

    tokio::time::advance(Duration::from_millis(20)).await;
    assert!(poll_once(request.as_mut()).await.is_none());
    tokio::time::advance(Duration::from_millis(9)).await;
    assert!(poll_once(request.as_mut()).await.is_none());
    assert_no_sent_frame(&mut controls);

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    tokio::time::advance(Duration::from_millis(1)).await;
    assert!(poll_once(request.as_mut()).await.is_none());
    let retry = controls.sent_rx.try_recv().unwrap();
    controls
        .response_tx
        .send(mbap_response(
            &retry,
            Bytes::from_static(&[0x03, 0x02, 0x00, 0x2A]),
        ))
        .unwrap();

    assert_eq!(request.await.unwrap(), vec![0x002A]);
    assert_no_sent_frame(&mut controls);
}

#[tokio::test(start_paused = true)]
async fn transport_timeout_retries_reads_but_not_writes() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        timeout: Duration::from_millis(50),
        retry: RetryConfig {
            max_retries: 1,
            retry_delay: Duration::from_millis(10),
            ..RetryConfig::default()
        },
        ..ClientConfig::default()
    };
    let client = ModbusClient::from_transport(sink, stream, config);

    controls.outcome_tx.send(SendOutcome::Timeout).unwrap();
    let mut read = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(read.as_mut()).await.is_none());
    assert_eq!(controls.sent_rx.try_recv().unwrap().pdu[0], 0x03);

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    tokio::time::advance(Duration::from_millis(10)).await;
    assert!(poll_once(read.as_mut()).await.is_none());
    let retry = controls.sent_rx.try_recv().unwrap();
    controls
        .response_tx
        .send(mbap_response(
            &retry,
            Bytes::from_static(&[0x03, 0x02, 0x00, 0x2A]),
        ))
        .unwrap();
    assert_eq!(read.await.unwrap(), vec![0x002A]);

    controls.outcome_tx.send(SendOutcome::Timeout).unwrap();
    let result = client.write_single_register(UnitId(1), 7, 0x1234).await;
    assert!(matches!(
        result,
        Err(ClientError::Transport(TransportError::Timeout))
    ));
    assert_eq!(controls.sent_rx.try_recv().unwrap().pdu[0], 0x06);
    tokio::time::advance(Duration::from_millis(10)).await;
    assert_no_sent_frame(&mut controls);
}

#[tokio::test(start_paused = true)]
async fn non_timeout_transport_errors_remain_terminal_for_reads() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        retry: RetryConfig {
            max_retries: 3,
            retry_delay: Duration::from_millis(10),
            ..RetryConfig::default()
        },
        ..default_config()
    };
    let client = ModbusClient::from_transport(sink, stream, config);

    controls.outcome_tx.send(SendOutcome::Failure).unwrap();
    let result = client.read_holding_registers(UnitId(1), 0, 1).await;
    assert!(matches!(
        result,
        Err(ClientError::Transport(TransportError::Io(_)))
    ));
    assert_eq!(controls.sent_rx.try_recv().unwrap().pdu[0], 0x03);
    tokio::time::advance(Duration::from_millis(30)).await;
    assert_no_sent_frame(&mut controls);
}

#[tokio::test(start_paused = true)]
async fn busy_write_retries_after_configured_delay_then_succeeds() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        timeout: Duration::from_millis(50),
        retry: RetryConfig {
            max_retries: 1,
            retry_delay: Duration::from_millis(10),
            ..RetryConfig::default()
        },
        ..ClientConfig::default()
    };
    let client = ModbusClient::from_transport(sink, stream, config);

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut write = Box::pin(client.write_single_register(UnitId(1), 7, 0x1234));
    assert!(poll_once(write.as_mut()).await.is_none());
    let first = controls.sent_rx.try_recv().unwrap();
    controls
        .response_tx
        .send(mbap_response(&first, Bytes::from_static(&[0x86, 0x06])))
        .unwrap();
    tokio::task::yield_now().await;
    assert!(poll_once(write.as_mut()).await.is_none());

    tokio::time::advance(Duration::from_millis(9)).await;
    assert!(poll_once(write.as_mut()).await.is_none());
    assert_no_sent_frame(&mut controls);
    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    tokio::time::advance(Duration::from_millis(1)).await;
    assert!(poll_once(write.as_mut()).await.is_none());
    let retry = controls.sent_rx.try_recv().unwrap();
    controls
        .response_tx
        .send(mbap_response(&retry, retry.pdu.clone()))
        .unwrap();

    write.await.unwrap();
    assert_no_sent_frame(&mut controls);
}

#[tokio::test(start_paused = true)]
async fn every_mutating_typed_function_sends_once_after_response_timeout() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        timeout: Duration::from_millis(20),
        retry: RetryConfig {
            max_retries: 3,
            retry_delay: Duration::from_millis(10),
            ..RetryConfig::default()
        },
        ..ClientConfig::default()
    };
    let client = ModbusClient::from_transport(sink, stream, config);

    assert_mutating_timeout_once(
        &mut controls,
        client.write_single_coil(UnitId(1), 0, true),
        0x05,
    )
    .await;
    assert_mutating_timeout_once(
        &mut controls,
        client.write_single_register(UnitId(1), 0, 1),
        0x06,
    )
    .await;
    assert_mutating_timeout_once(
        &mut controls,
        client.write_multiple_coils(UnitId(1), 0, &[true, false]),
        0x0F,
    )
    .await;
    assert_mutating_timeout_once(
        &mut controls,
        client.write_multiple_registers(UnitId(1), 0, &[1, 2]),
        0x10,
    )
    .await;
    assert_mutating_timeout_once(
        &mut controls,
        client.write_file_record(UnitId(1), &[0x06, 0, 1, 0, 0, 0, 1, 0, 1]),
        0x15,
    )
    .await;
    assert_mutating_timeout_once(
        &mut controls,
        client.mask_write_register(UnitId(1), 0, 0xFF00, 0x00FF),
        0x16,
    )
    .await;
    assert_mutating_timeout_once(
        &mut controls,
        client.read_write_multiple_registers(UnitId(1), 0, 1, 0, &[1]),
        0x17,
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn retry_cap_and_total_operation_envelope_are_exact() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        timeout: Duration::from_millis(20),
        retry: RetryConfig {
            max_retries: 2,
            retry_delay: Duration::from_millis(10),
            ..RetryConfig::default()
        },
        ..ClientConfig::default()
    };
    let client = ModbusClient::from_transport(sink, stream, config);
    let start = tokio::time::Instant::now();
    let mut request = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));

    for attempt in 1..=3 {
        controls.outcome_tx.send(SendOutcome::Success).unwrap();
        assert!(poll_once(request.as_mut()).await.is_none());
        assert_eq!(controls.sent_rx.try_recv().unwrap().pdu[0], 0x03);
        tokio::time::advance(Duration::from_millis(20)).await;
        let result = poll_once(request.as_mut()).await;
        if attempt < 3 {
            assert!(result.is_none());
            tokio::time::advance(Duration::from_millis(10)).await;
        } else {
            assert!(matches!(
                result,
                Some(Err(ClientError::RetriesExhausted {
                    attempts: 3,
                    last_error,
                })) if matches!(*last_error, ClientError::Timeout)
            ));
        }
    }

    assert_eq!(
        tokio::time::Instant::now().duration_since(start),
        Duration::from_millis(80)
    );
    assert_no_sent_frame(&mut controls);
}

#[tokio::test(start_paused = true)]
async fn logical_operation_holds_one_admission_permit_across_backoff() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        timeout: Duration::from_millis(20),
        max_in_flight: 1,
        retry: RetryConfig {
            max_retries: 1,
            retry_delay: Duration::from_millis(10),
            ..RetryConfig::default()
        },
        ..ClientConfig::default()
    };
    let client = ModbusClient::from_transport(sink, stream, config);

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut first = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(first.as_mut()).await.is_none());
    assert_eq!(controls.sent_rx.try_recv().unwrap().pdu[0], 0x03);
    tokio::time::advance(Duration::from_millis(20)).await;
    assert!(poll_once(first.as_mut()).await.is_none());

    let mut waiting = Box::pin(client.read_input_registers(UnitId(1), 0, 1));
    assert!(poll_once(waiting.as_mut()).await.is_none());
    assert_no_sent_frame(&mut controls);

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    tokio::time::advance(Duration::from_millis(10)).await;
    assert!(poll_once(first.as_mut()).await.is_none());
    let retry = controls.sent_rx.try_recv().unwrap();
    controls
        .response_tx
        .send(mbap_response(
            &retry,
            Bytes::from_static(&[0x03, 0x02, 0x00, 0x2A]),
        ))
        .unwrap();
    assert_eq!(first.await.unwrap(), vec![0x002A]);

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    assert!(poll_once(waiting.as_mut()).await.is_none());
    let sent = controls.sent_rx.try_recv().unwrap();
    assert_eq!(sent.pdu[0], 0x04);
    controls
        .response_tx
        .send(mbap_response(
            &sent,
            Bytes::from_static(&[0x04, 0x02, 0x00, 0x11]),
        ))
        .unwrap();
    assert_eq!(waiting.await.unwrap(), vec![0x0011]);
}

#[tokio::test(start_paused = true)]
async fn expired_sink_wait_does_not_send_orphan_request() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        timeout: Duration::from_millis(20),
        max_in_flight: 2,
        retry: RetryConfig {
            max_retries: 0,
            ..RetryConfig::default()
        },
        ..ClientConfig::default()
    };
    let client = ModbusClient::from_transport(sink, stream, config);

    let mut holding_sink = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(holding_sink.as_mut()).await.is_none());
    assert_eq!(controls.sent_rx.try_recv().unwrap().pdu[0], 0x03);

    let mut waiting = Box::pin(client.write_single_register(UnitId(1), 0, 1));
    assert!(poll_once(waiting.as_mut()).await.is_none());
    tokio::time::advance(Duration::from_millis(20)).await;
    assert!(matches!(
        poll_once(waiting.as_mut()).await,
        Some(Err(ClientError::Timeout))
    ));
    assert_no_sent_frame(&mut controls);

    drop(holding_sink);
    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    tokio::time::advance(Duration::from_millis(1)).await;
    assert_no_sent_frame(&mut controls);
}

#[tokio::test(start_paused = true)]
async fn cancelled_request_releases_permit_and_deadline_reclaims_slot() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        timeout: Duration::from_millis(20),
        max_in_flight: 1,
        retry: RetryConfig {
            max_retries: 0,
            ..RetryConfig::default()
        },
        ..ClientConfig::default()
    };
    let client = ModbusClient::from_transport(sink, stream, config);

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut cancelled = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(cancelled.as_mut()).await.is_none());
    assert_eq!(controls.sent_rx.try_recv().unwrap().pdu[0], 0x03);
    drop(cancelled);

    tokio::time::advance(Duration::from_millis(20)).await;
    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut next = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(next.as_mut()).await.is_none());
    let sent = controls.sent_rx.try_recv().unwrap();
    controls
        .response_tx
        .send(mbap_response(
            &sent,
            Bytes::from_static(&[0x03, 0x02, 0x00, 0x2A]),
        ))
        .unwrap();
    assert_eq!(next.await.unwrap(), vec![0x002A]);
}

#[tokio::test(start_paused = true)]
async fn rtu_client_uses_same_read_and_write_retry_classification() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        timeout: Duration::from_millis(20),
        retry: RetryConfig {
            max_retries: 1,
            retry_delay: Duration::from_millis(10),
            ..RetryConfig::default()
        },
        ..ClientConfig::default()
    };
    let client = ModbusClient::from_rtu_transport(sink, stream, config);

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut read = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(read.as_mut()).await.is_none());
    assert_eq!(controls.sent_rx.try_recv().unwrap().pdu[0], 0x03);
    tokio::time::advance(Duration::from_millis(20)).await;
    assert!(poll_once(read.as_mut()).await.is_none());
    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    tokio::time::advance(Duration::from_millis(10)).await;
    assert!(poll_once(read.as_mut()).await.is_none());
    assert_eq!(controls.sent_rx.try_recv().unwrap().pdu[0], 0x03);
    controls
        .response_tx
        .send(rtu_frame(1, Bytes::from_static(&[0x03, 0x02, 0x00, 0x2A])))
        .unwrap();
    assert_eq!(read.await.unwrap(), vec![0x002A]);

    assert_mutating_timeout_once(
        &mut controls,
        client.write_single_register(UnitId(1), 0, 1),
        0x06,
    )
    .await;
}

#[tokio::test]
async fn shutdown_cancels_pending() {
    let addr = start_register_server().await;
    let client = std::sync::Arc::new(ModbusClient::connect(addr, default_config()).await.unwrap());

    client.shutdown().await;

    let result = client.read_holding_registers(UnitId(0xFF), 0, 1).await;
    assert!(matches!(result, Err(ClientError::NotConnected)));
}

#[tokio::test]
async fn zero_active_shutdown_joins_background_tasks_before_return() {
    let (sink, stream, controls) = controlled_transport();
    let client = ModbusClient::from_transport(sink, stream, default_config());

    client.shutdown().await;

    assert!(
        controls
            .stream_dropped
            .load(std::sync::atomic::Ordering::SeqCst)
    );
}

#[tokio::test]
async fn shutdown_keeps_reader_alive_while_admitted_request_drains() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        shutdown_timeout: Duration::from_secs(1),
        ..default_config()
    };
    let client = Arc::new(ModbusClient::from_transport(sink, stream, config));

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let request_client = Arc::clone(&client);
    let request =
        tokio::spawn(async move { request_client.read_holding_registers(UnitId(1), 0, 1).await });
    let sent = controls.sent_rx.recv().await.unwrap();

    let shutdown_client = Arc::clone(&client);
    let shutdown = tokio::spawn(async move { shutdown_client.shutdown().await });
    tokio::task::yield_now().await;
    assert!(!shutdown.is_finished());

    controls
        .response_tx
        .send(mbap_response(
            &sent,
            Bytes::from_static(&[0x03, 0x02, 0x00, 0x2A]),
        ))
        .unwrap();
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    assert!(request.is_finished());
    assert_eq!(request.await.unwrap().unwrap(), vec![0x002A]);
    shutdown.await.unwrap();
}

#[tokio::test]
async fn shutdown_wakes_preseal_admission_waiter_without_sending() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        max_in_flight: 1,
        shutdown_timeout: Duration::from_secs(1),
        ..default_config()
    };
    let client = Arc::new(ModbusClient::from_transport(sink, stream, config));

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let first_client = Arc::clone(&client);
    let first =
        tokio::spawn(async move { first_client.read_holding_registers(UnitId(1), 0, 1).await });
    let _sent = controls.sent_rx.recv().await.unwrap();

    let waiting_client = Arc::clone(&client);
    let waiting =
        tokio::spawn(async move { waiting_client.read_input_registers(UnitId(1), 0, 1).await });
    tokio::task::yield_now().await;
    assert_no_sent_frame(&mut controls);

    let shutdown_client = Arc::clone(&client);
    let shutdown = tokio::spawn(async move { shutdown_client.shutdown().await });
    tokio::task::yield_now().await;

    assert!(waiting.is_finished());
    assert!(matches!(
        waiting.await.unwrap(),
        Err(ClientError::ShuttingDown)
    ));
    assert_no_sent_frame(&mut controls);

    let rejected = client.read_holding_registers(UnitId(1), 0, 1).await;
    assert!(matches!(rejected, Err(ClientError::NotConnected)));
    assert_no_sent_frame(&mut controls);

    client.abort();
    assert!(matches!(
        first.await.unwrap(),
        Err(ClientError::ShuttingDown)
    ));
    shutdown.await.unwrap();
}

#[tokio::test]
async fn shutdown_drains_broadcast_without_a_transaction_slot() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        shutdown_timeout: Duration::from_secs(1),
        ..default_config()
    };
    let client = Arc::new(ModbusClient::from_transport(sink, stream, config));

    let broadcast_client = Arc::clone(&client);
    let broadcast = tokio::spawn(async move {
        broadcast_client
            .write_single_register(UnitId(0), 1, 1)
            .await
    });
    assert_eq!(controls.sent_rx.recv().await.unwrap().unit_id(), 0);

    let shutdown_client = Arc::clone(&client);
    let shutdown = tokio::spawn(async move { shutdown_client.shutdown().await });
    tokio::task::yield_now().await;
    assert!(!shutdown.is_finished());

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    broadcast.await.unwrap().unwrap();
    shutdown.await.unwrap();
}

#[tokio::test]
async fn dropping_registered_requests_reclaims_exact_slots_immediately() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        timeout: Duration::from_secs(60),
        max_in_flight: 1,
        retry: RetryConfig {
            max_retries: 0,
            ..RetryConfig::default()
        },
        ..ClientConfig::default()
    };
    let client = ModbusClient::from_transport(sink, stream, config);

    for _ in 0..16 {
        controls.outcome_tx.send(SendOutcome::Success).unwrap();
        let mut request = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
        assert!(poll_once(request.as_mut()).await.is_none());
        controls.sent_rx.try_recv().unwrap();
        drop(request);
    }

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut next = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(next.as_mut()).await.is_none());
    let sent = controls.sent_rx.try_recv().unwrap();
    controls
        .response_tx
        .send(mbap_response(
            &sent,
            Bytes::from_static(&[0x03, 0x02, 0x00, 0x2A]),
        ))
        .unwrap();
    assert_eq!(next.await.unwrap(), vec![0x002A]);
}

#[tokio::test(start_paused = true)]
async fn shutdown_deadline_hard_cancels_registered_request_and_ignores_late_response() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        timeout: Duration::from_secs(60),
        shutdown_timeout: Duration::from_millis(20),
        retry: RetryConfig {
            max_retries: 0,
            ..RetryConfig::default()
        },
        ..ClientConfig::default()
    };
    let client = Arc::new(ModbusClient::from_transport(sink, stream, config));

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let request_client = Arc::clone(&client);
    let request =
        tokio::spawn(async move { request_client.read_holding_registers(UnitId(1), 0, 1).await });
    let sent = controls.sent_rx.recv().await.unwrap();

    let shutdown_client = Arc::clone(&client);
    let shutdown = tokio::spawn(async move { shutdown_client.shutdown().await });
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(19)).await;
    assert!(!request.is_finished());
    assert!(!shutdown.is_finished());

    tokio::time::advance(Duration::from_millis(1)).await;
    tokio::task::yield_now().await;
    assert!(matches!(
        request.await.unwrap(),
        Err(ClientError::ShuttingDown)
    ));
    shutdown.await.unwrap();
    assert!(
        controls
            .response_tx
            .send(mbap_response(
                &sent,
                Bytes::from_static(&[0x03, 0x02, 0x00, 0x2A]),
            ))
            .is_err()
    );
}

#[tokio::test]
async fn abort_cancels_sink_send_and_sink_mutex_wait_without_a_second_frame() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        max_in_flight: 2,
        ..default_config()
    };
    let client = Arc::new(ModbusClient::from_transport(sink, stream, config));

    let sending_client = Arc::clone(&client);
    let sending =
        tokio::spawn(async move { sending_client.write_single_register(UnitId(1), 0, 1).await });
    assert_eq!(controls.sent_rx.recv().await.unwrap().pdu[0], 0x06);

    let waiting_client = Arc::clone(&client);
    let waiting =
        tokio::spawn(async move { waiting_client.read_holding_registers(UnitId(1), 0, 1).await });
    tokio::task::yield_now().await;
    assert_no_sent_frame(&mut controls);

    client.abort();
    assert!(matches!(
        sending.await.unwrap(),
        Err(ClientError::ShuttingDown)
    ));
    assert!(matches!(
        waiting.await.unwrap(),
        Err(ClientError::ShuttingDown)
    ));
    assert_no_sent_frame(&mut controls);
    client.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn abort_during_busy_backoff_prevents_retry() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        timeout: Duration::from_secs(1),
        retry: RetryConfig {
            max_retries: 3,
            retry_delay: Duration::from_secs(30),
            ..RetryConfig::default()
        },
        ..ClientConfig::default()
    };
    let client = Arc::new(ModbusClient::from_transport(sink, stream, config));

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let request_client = Arc::clone(&client);
    let request =
        tokio::spawn(async move { request_client.read_holding_registers(UnitId(1), 0, 1).await });
    let sent = controls.sent_rx.recv().await.unwrap();
    controls
        .response_tx
        .send(mbap_response(&sent, Bytes::from_static(&[0x83, 0x06])))
        .unwrap();
    tokio::task::yield_now().await;

    client.abort();
    assert!(matches!(
        request.await.unwrap(),
        Err(ClientError::ShuttingDown)
    ));
    tokio::time::advance(Duration::from_secs(30)).await;
    assert_no_sent_frame(&mut controls);
    client.shutdown().await;
}

#[tokio::test]
async fn abort_hard_cancels_broadcast_after_frame_observation() {
    let (sink, stream, mut controls) = controlled_transport();
    let client = Arc::new(ModbusClient::from_transport(sink, stream, default_config()));

    let broadcast_client = Arc::clone(&client);
    let broadcast = tokio::spawn(async move {
        broadcast_client
            .write_single_register(UnitId(0), 1, 1)
            .await
    });
    assert_eq!(controls.sent_rx.recv().await.unwrap().unit_id(), 0);

    client.abort();
    assert!(matches!(
        broadcast.await.unwrap(),
        Err(ClientError::ShuttingDown)
    ));
    client.shutdown().await;
}

#[tokio::test]
async fn concurrent_shutdown_callers_share_coordinator_when_one_caller_is_cancelled() {
    let (sink, stream, mut controls) = controlled_transport();
    let client = Arc::new(ModbusClient::from_transport(sink, stream, default_config()));

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let request_client = Arc::clone(&client);
    let request =
        tokio::spawn(async move { request_client.read_holding_registers(UnitId(1), 0, 1).await });
    let sent = controls.sent_rx.recv().await.unwrap();

    let mut shutdowns = Vec::new();
    for _ in 0..32 {
        let shutdown_client = Arc::clone(&client);
        shutdowns.push(tokio::spawn(async move {
            shutdown_client.shutdown().await;
        }));
    }
    tokio::task::yield_now().await;
    shutdowns.remove(0).abort();

    controls
        .response_tx
        .send(mbap_response(
            &sent,
            Bytes::from_static(&[0x03, 0x02, 0x00, 0x2A]),
        ))
        .unwrap();
    assert_eq!(request.await.unwrap().unwrap(), vec![0x002A]);
    for shutdown in shutdowns {
        shutdown.await.unwrap();
    }
    assert!(!client.is_connected());
}

#[tokio::test]
async fn abort_is_idempotent_rejects_new_calls_and_shutdown_still_joins() {
    let (sink, stream, mut controls) = controlled_transport();
    let client = ModbusClient::from_transport(sink, stream, default_config());

    client.abort();
    client.abort();
    let result = client.read_holding_registers(UnitId(1), 0, 1).await;
    assert!(matches!(result, Err(ClientError::NotConnected)));
    assert_no_sent_frame(&mut controls);

    client.shutdown().await;
    client.shutdown().await;
    assert!(
        controls
            .stream_dropped
            .load(std::sync::atomic::Ordering::SeqCst)
    );
}

#[tokio::test]
async fn rtu_abort_uses_the_generic_cancellation_path() {
    let (sink, stream, mut controls) = controlled_transport();
    let client = Arc::new(ModbusClient::from_rtu_transport(
        sink,
        stream,
        default_config(),
    ));

    let request_client = Arc::clone(&client);
    let request =
        tokio::spawn(async move { request_client.read_holding_registers(UnitId(1), 0, 1).await });
    assert_eq!(controls.sent_rx.recv().await.unwrap().unit_id(), 1);
    client.abort();
    assert!(matches!(
        request.await.unwrap(),
        Err(ClientError::ShuttingDown)
    ));
    client.shutdown().await;
}

#[test]
fn abort_and_drop_after_runtime_shutdown_do_not_panic() {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sink, stream, _controls) = controlled_transport();
        let client = runtime
            .block_on(async { ModbusClient::from_transport(sink, stream, default_config()) });
        drop(runtime);
        client.abort();
        drop(client);
    }));

    assert!(result.is_ok());
}

#[test]
fn drop_outside_runtime_requests_background_task_cancellation() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (sink, stream, controls) = controlled_transport();
    let stream_dropped = Arc::clone(&controls.stream_dropped);
    let client =
        runtime.block_on(async { ModbusClient::from_transport(sink, stream, default_config()) });

    std::thread::spawn(move || drop(client)).join().unwrap();

    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), async {
            while !stream_dropped.load(std::sync::atomic::Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("final-owner Drop should abort the reader task");
    });
}

#[tokio::test]
async fn invalid_request_arguments_return_encode_errors_before_send() {
    let addr = start_register_server().await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    assert_quantity_encode_error(client.read_coils(UnitId(0xFF), 0, 2001).await, 2001);
    assert_quantity_encode_error(client.read_holding_registers(UnitId(0xFF), 0, 0).await, 0);
    assert_quantity_encode_error(client.read_input_registers(UnitId(0xFF), 0, 126).await, 126);

    assert_quantity_encode_error(client.write_multiple_coils(UnitId(0xFF), 0, &[]).await, 0);
    let too_many_coils = vec![false; 1969];
    assert_quantity_encode_error(
        client
            .write_multiple_coils(UnitId(0xFF), 0, &too_many_coils)
            .await,
        1969,
    );

    let too_many_registers = vec![0; 124];
    assert_quantity_encode_error(
        client
            .write_multiple_registers(UnitId(0xFF), 0, &too_many_registers)
            .await,
        124,
    );

    assert_quantity_encode_error(
        client
            .read_write_multiple_registers(UnitId(0xFF), 0, 126, 0, &[0x0001])
            .await,
        126,
    );

    assert_file_byte_count_encode_error(
        client.read_file_record(UnitId(0xFF), &[0; 6]).await,
        6,
        7,
        245,
    );
    assert_file_byte_count_encode_error(
        client.write_file_record(UnitId(0xFF), &[0; 246]).await,
        246,
        7,
        245,
    );
}

// ---------------------------------------------------------------------------
// Client/transport correctness regression tests
// ---------------------------------------------------------------------------

/// Start a server that builds each response PDU from a closure of
/// `(function_code, request_pdu)`. Lets a test script malformed/adversarial
/// responses (short byte counts, wrong function codes, ...).
async fn start_scripted_server<F>(respond: F) -> SocketAddr
where
    F: Fn(u8, &[u8]) -> Vec<u8> + Send + Sync + 'static,
{
    let respond = std::sync::Arc::new(respond);
    let listener =
        TcpServerListener::bind("127.0.0.1:0".parse().unwrap(), TcpServerConfig::default())
            .await
            .unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        while let Ok((mut sink, mut stream, _, _guard)) = listener.accept().await {
            let respond = std::sync::Arc::clone(&respond);
            tokio::spawn(async move {
                while let Ok(req) = stream.recv().await {
                    let txn_id = match req.header {
                        FrameHeader::Mbap(h) => h.transaction_id.get(),
                        FrameHeader::Rtu { .. } => 0,
                    };
                    let unit_id = req.unit_id();
                    let resp_pdu = respond(req.pdu[0], &req.pdu);
                    let header = MbapHeader::new(txn_id, unit_id, resp_pdu.len() as u16);
                    let resp = Frame {
                        header: FrameHeader::Mbap(header),
                        pdu: Bytes::from(resp_pdu),
                    };
                    if sink.send(resp).await.is_err() {
                        break;
                    }
                }
            });
        }
    });

    addr
}

fn device_id_basic_response(more_follows: bool, next_object_id: u8, object_id: u8) -> Vec<u8> {
    vec![
        0x2B,
        0x0E,
        0x01,
        0x81,
        if more_follows { 0xFF } else { 0x00 },
        if more_follows { next_object_id } else { 0x00 },
        0x01,
        object_id,
        0x00,
    ]
}

/// The background reader must survive a benign idle read-timeout. `connect()`
/// maps `config.timeout` onto the transport read-timeout, so an idle period
/// longer than `timeout` previously killed the reader (cancel_all + break).
#[tokio::test]
async fn idle_reader_survives_read_timeout() {
    let addr = start_register_server().await;
    let config = ClientConfig {
        timeout: Duration::from_millis(300),
        ..ClientConfig::default()
    };
    let client = ModbusClient::connect(addr, config).await.unwrap();

    // First request works.
    let regs = client
        .read_holding_registers(UnitId(0xFF), 0, 2)
        .await
        .unwrap();
    assert_eq!(regs, vec![0x1234, 0x5678]);

    // Idle well past the read timeout — the reader must NOT tear down a healthy
    // connection (several recv() timeouts elapse during this window).
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(
        client.is_connected(),
        "reader died on a benign idle read-timeout"
    );

    // A request after the idle period still succeeds.
    let regs = client
        .read_holding_registers(UnitId(0xFF), 0, 2)
        .await
        .unwrap();
    assert_eq!(regs, vec![0x1234, 0x5678]);
}

/// A server returning fewer coil bytes than the requested quantity needs must
/// produce a clean error, not an out-of-bounds panic in `coil(i)`.
#[tokio::test]
async fn short_coil_response_is_error_not_panic() {
    // FC01 reply with byte_count = 1 regardless of how many coils were asked.
    let addr = start_scripted_server(|fc, _req| {
        if fc == 0x01 {
            vec![0x01, 0x01, 0xFF]
        } else {
            vec![fc | 0x80, 0x01]
        }
    })
    .await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    // Request 64 coils (needs 8 bytes); server returns 1.
    let result = client.read_coils(UnitId(0xFF), 0, 64).await;
    assert!(
        matches!(result, Err(ClientError::ShortResponse { .. })),
        "expected ShortResponse, got {result:?}"
    );
}

/// A response whose function code does not echo the request (but lands on the
/// matching transaction slot) must be rejected, not silently misinterpreted.
#[tokio::test]
async fn wrong_function_code_is_rejected() {
    // Answer an FC03 request with an FC04 response body.
    let addr = start_scripted_server(|fc, _req| {
        if fc == 0x03 {
            vec![0x04, 0x02, 0x00, 0x42]
        } else {
            vec![fc | 0x80, 0x01]
        }
    })
    .await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    let result = client.read_holding_registers(UnitId(0xFF), 0, 1).await;
    assert!(
        matches!(
            result,
            Err(ClientError::UnexpectedResponse {
                expected: 0x03,
                got: 0x04
            })
        ),
        "expected UnexpectedResponse, got {result:?}"
    );
}

/// A register read whose response is shorter than the requested quantity must
/// error instead of silently returning a truncated vector.
#[tokio::test]
async fn short_register_response_is_error() {
    // FC03 reply with byte_count = 2 (one register) regardless of the request.
    let addr = start_scripted_server(|fc, _req| {
        if fc == 0x03 {
            vec![0x03, 0x02, 0x00, 0x42]
        } else {
            vec![fc | 0x80, 0x01]
        }
    })
    .await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    let result = client.read_holding_registers(UnitId(0xFF), 0, 10).await;
    assert!(
        matches!(result, Err(ClientError::ShortResponse { .. })),
        "expected ShortResponse, got {result:?}"
    );
}

#[tokio::test]
async fn read_discrete_inputs_happy_path() {
    // FC02 reply: byte_count=1, bits 0b0001_0101 (inputs 0,2,4 set).
    let addr = start_scripted_server(|fc, _req| {
        if fc == 0x02 {
            vec![0x02, 0x01, 0x15]
        } else {
            vec![fc | 0x80, 0x01]
        }
    })
    .await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    let inputs = client
        .read_discrete_inputs(UnitId(0xFF), 0, 5)
        .await
        .unwrap();
    assert_eq!(inputs, vec![true, false, true, false, true]);
}

#[tokio::test]
async fn short_discrete_input_response_is_error_not_panic() {
    // FC02 reply with byte_count=1 while 64 inputs were requested.
    let addr = start_scripted_server(|fc, _req| {
        if fc == 0x02 {
            vec![0x02, 0x01, 0xFF]
        } else {
            vec![fc | 0x80, 0x01]
        }
    })
    .await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    let result = client.read_discrete_inputs(UnitId(0xFF), 0, 64).await;
    assert!(
        matches!(result, Err(ClientError::ShortResponse { .. })),
        "expected ShortResponse, got {result:?}"
    );
}

#[tokio::test]
async fn exception_with_mismatched_fc_is_rejected() {
    // Answer an FC03 request with an exception flagged for FC04 (0x84). The FC
    // echo check must reject it rather than surfacing it as the request's
    // exception.
    let addr = start_scripted_server(|fc, _req| {
        if fc == 0x03 {
            vec![0x84, 0x01]
        } else {
            vec![fc | 0x80, 0x01]
        }
    })
    .await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    let result = client.read_holding_registers(UnitId(0xFF), 0, 1).await;
    assert!(
        matches!(
            result,
            Err(ClientError::UnexpectedResponse {
                expected: 0x03,
                got: 0x84
            })
        ),
        "expected UnexpectedResponse, got {result:?}"
    );
}

#[tokio::test]
async fn matching_exception_surfaces_as_exception() {
    // An exception that DOES echo the request FC (0x83 for FC03, non-retryable
    // IllegalDataAddress 0x02) must pass the FC check and surface as Exception.
    let addr = start_scripted_server(|fc, _req| {
        if fc == 0x03 {
            vec![0x83, 0x02]
        } else {
            vec![fc | 0x80, 0x01]
        }
    })
    .await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    let result = client.read_holding_registers(UnitId(0xFF), 0, 1).await;
    match result {
        Err(ClientError::Exception(exc)) => assert_eq!(exc.function_code.code(), 0x03),
        other => panic!("expected Exception for FC 0x03, got {other:?}"),
    }
}

#[tokio::test]
async fn write_single_register_rejects_mismatched_echo() {
    // FC06 response must echo the requested address and value.
    let addr = start_scripted_server(|fc, _req| {
        if fc == 0x06 {
            vec![0x06, 0x00, 0x02, 0xBE, 0xEF]
        } else {
            vec![fc | 0x80, 0x01]
        }
    })
    .await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    assert_echo_mismatch(
        client
            .write_single_register(UnitId(0xFF), 0x0001, 0xBEEF)
            .await,
        "address",
        0x0001,
        0x0002,
    );
}

#[tokio::test]
async fn write_single_coil_rejects_mismatched_echo() {
    // FC05 response must echo 0xFF00 for an ON write.
    let addr = start_scripted_server(|fc, _req| {
        if fc == 0x05 {
            vec![0x05, 0x00, 0x05, 0x00, 0x00]
        } else {
            vec![fc | 0x80, 0x01]
        }
    })
    .await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    assert_echo_mismatch(
        client.write_single_coil(UnitId(0xFF), 0x0005, true).await,
        "value",
        0xFF00,
        0x0000,
    );
}

#[tokio::test]
async fn write_multiple_coils_rejects_mismatched_echo() {
    // FC0F response must echo the starting address and quantity written.
    let addr = start_scripted_server(|fc, _req| {
        if fc == 0x0F {
            vec![0x0F, 0x00, 0x10, 0x00, 0x03]
        } else {
            vec![fc | 0x80, 0x01]
        }
    })
    .await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    assert_echo_mismatch(
        client
            .write_multiple_coils(UnitId(0xFF), 0x0010, &[true, false])
            .await,
        "quantity",
        0x0002,
        0x0003,
    );
}

#[tokio::test]
async fn write_multiple_registers_rejects_mismatched_echo() {
    // FC10 response must echo the starting address and quantity written.
    let addr = start_scripted_server(|fc, _req| {
        if fc == 0x10 {
            vec![0x10, 0x00, 0x20, 0x00, 0x01]
        } else {
            vec![fc | 0x80, 0x01]
        }
    })
    .await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    assert_echo_mismatch(
        client
            .write_multiple_registers(UnitId(0xFF), 0x0020, &[0x0001, 0x0002])
            .await,
        "quantity",
        0x0002,
        0x0001,
    );
}

#[tokio::test]
async fn mask_write_register_rejects_mismatched_echo() {
    // FC16 response must echo address, AND mask, and OR mask.
    let addr = start_scripted_server(|fc, _req| {
        if fc == 0x16 {
            vec![0x16, 0x00, 0x04, 0x00, 0xF2, 0x00, 0x24]
        } else {
            vec![fc | 0x80, 0x01]
        }
    })
    .await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    assert_echo_mismatch(
        client
            .mask_write_register(UnitId(0xFF), 0x0004, 0x00F2, 0x0025)
            .await,
        "or_mask",
        0x0025,
        0x0024,
    );
}

#[tokio::test]
async fn short_input_register_response_is_error() {
    // FC04 reply with byte_count=2 (one register) while 10 were requested.
    let addr = start_scripted_server(|fc, _req| {
        if fc == 0x04 {
            vec![0x04, 0x02, 0x00, 0x42]
        } else {
            vec![fc | 0x80, 0x01]
        }
    })
    .await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    let result = client.read_input_registers(UnitId(0xFF), 0, 10).await;
    assert!(
        matches!(result, Err(ClientError::ShortResponse { .. })),
        "expected ShortResponse, got {result:?}"
    );
}

#[tokio::test]
async fn short_read_write_registers_response_is_error() {
    // FC17 reply with byte_count=2 while read_quantity=10 — the guard must key
    // on the read quantity, not the write quantity.
    let addr = start_scripted_server(|fc, _req| {
        if fc == 0x17 {
            vec![0x17, 0x02, 0x00, 0x42]
        } else {
            vec![fc | 0x80, 0x01]
        }
    })
    .await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    let result = client
        .read_write_multiple_registers(UnitId(0xFF), 0, 10, 0, &[0x0001])
        .await;
    assert!(
        matches!(result, Err(ClientError::ShortResponse { .. })),
        "expected ShortResponse, got {result:?}"
    );
}

#[tokio::test]
async fn short_raw_register_response_is_error() {
    // The zero-copy raw variant must apply the same short-response guard.
    let addr = start_scripted_server(|fc, _req| {
        if fc == 0x03 {
            vec![0x03, 0x02, 0x00, 0x42]
        } else {
            vec![fc | 0x80, 0x01]
        }
    })
    .await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    let result = client.read_holding_registers_raw(UnitId(0xFF), 0, 10).await;
    assert!(
        matches!(result, Err(ClientError::ShortResponse { .. })),
        "expected ShortResponse, got {result:?}"
    );
}

#[tokio::test]
async fn typed_reads_reject_overlong_success_responses() {
    let addr = start_scripted_server(|fc, _req| match fc {
        0x01 | 0x02 => vec![fc, 0x02, 0x00, 0x00],
        0x03 | 0x04 | 0x17 => vec![fc, 0x04, 0x00, 0x01, 0x00, 0x02],
        _ => vec![fc | 0x80, 0x01],
    })
    .await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    assert_unexpected_length(client.read_coils(UnitId(1), 0, 1).await, 0x01, 1, 2);
    assert_unexpected_length(
        client.read_discrete_inputs(UnitId(1), 0, 1).await,
        0x02,
        1,
        2,
    );
    assert_unexpected_length(
        client.read_holding_registers(UnitId(1), 0, 1).await,
        0x03,
        2,
        4,
    );
    assert_unexpected_length(
        client.read_holding_registers_raw(UnitId(1), 0, 1).await,
        0x03,
        2,
        4,
    );
    assert_unexpected_length(
        client.read_input_registers(UnitId(1), 0, 1).await,
        0x04,
        2,
        4,
    );
    assert_unexpected_length(
        client
            .read_write_multiple_registers(UnitId(1), 0, 1, 0, &[0x0001, 0x0002])
            .await,
        0x17,
        2,
        4,
    );
}

#[tokio::test]
async fn typed_bit_reads_reject_nonzero_final_padding() {
    let addr = start_scripted_server(|fc, _req| match fc {
        0x01 | 0x02 => vec![fc, 0x01, 0x80],
        _ => vec![fc | 0x80, 0x01],
    })
    .await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    assert_unexpected_padding(client.read_coils(UnitId(1), 0, 1).await, 0x01, 0xFE, 0x80);
    assert_unexpected_padding(
        client.read_discrete_inputs(UnitId(1), 0, 1).await,
        0x02,
        0xFE,
        0x80,
    );
}

#[tokio::test]
async fn raw_holding_register_response_keeps_the_owned_payload_slice() {
    let (sink, stream, mut controls) = controlled_transport();
    let client = ModbusClient::from_transport(sink, stream, default_config());
    let response_pdu = Bytes::from_static(&[0x03, 0x02, 0x12, 0x34]);
    let expected_data_ptr = response_pdu[2..].as_ptr();

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut request = Box::pin(client.read_holding_registers_raw(UnitId(1), 0, 1));
    assert!(poll_once(request.as_mut()).await.is_none());
    let sent = controls.sent_rx.try_recv().unwrap();
    let txn_id = match sent.header {
        FrameHeader::Mbap(header) => header.transaction_id.get(),
        FrameHeader::Rtu { .. } => panic!("expected MBAP request"),
    };
    controls
        .response_tx
        .send(Frame {
            header: FrameHeader::Mbap(MbapHeader::new(txn_id, 1, 4)),
            pdu: response_pdu,
        })
        .unwrap();

    let raw = request.await.unwrap();
    assert_eq!(raw, Bytes::from_static(&[0x12, 0x34]));
    assert_eq!(raw.as_ptr(), expected_data_ptr);
}

#[tokio::test]
async fn typed_register_reads_reject_odd_payloads_before_materialization() {
    let addr = start_scripted_server(|fc, _req| match fc {
        0x03 | 0x04 | 0x17 => vec![fc, 0x03, 0x00, 0x01, 0x02],
        _ => vec![fc | 0x80, 0x01],
    })
    .await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    let results = [
        client
            .read_holding_registers(UnitId(1), 0, 1)
            .await
            .map(|_| ()),
        client
            .read_holding_registers_raw(UnitId(1), 0, 1)
            .await
            .map(|_| ()),
        client
            .read_input_registers(UnitId(1), 0, 1)
            .await
            .map(|_| ()),
        client
            .read_write_multiple_registers(UnitId(1), 0, 1, 0, &[0x0001])
            .await
            .map(|_| ()),
    ];

    for result in results {
        assert!(matches!(
            result,
            Err(ClientError::Codec(
                rusty_modbus_codec::DecodeError::InvalidRegisterDataLength { length: 3 }
            ))
        ));
    }
}

#[tokio::test]
async fn response_shape_errors_are_not_retried() {
    let (sink, stream, mut controls) = controlled_transport();
    let config = ClientConfig {
        retry: RetryConfig {
            max_retries: 3,
            ..RetryConfig::default()
        },
        ..default_config()
    };
    let client = ModbusClient::from_transport(sink, stream, config);

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut request = Box::pin(client.read_coils(UnitId(1), 0, 1));
    assert!(poll_once(request.as_mut()).await.is_none());
    let sent = controls.sent_rx.try_recv().unwrap();
    let txn_id = match sent.header {
        FrameHeader::Mbap(header) => header.transaction_id.get(),
        FrameHeader::Rtu { .. } => panic!("expected MBAP request"),
    };
    controls
        .response_tx
        .send(Frame {
            header: FrameHeader::Mbap(MbapHeader::new(txn_id, 1, 4)),
            pdu: Bytes::from_static(&[0x01, 0x02, 0x00, 0x00]),
        })
        .unwrap();

    assert_unexpected_length(request.await, 0x01, 1, 2);
    assert!(matches!(
        controls.sent_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn rtu_typed_reads_use_shared_response_shape_validation() {
    let (sink, stream, mut controls) = controlled_transport();
    let client = ModbusClient::from_rtu_transport(sink, stream, default_config());

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut request = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(request.as_mut()).await.is_none());
    assert_eq!(controls.sent_rx.try_recv().unwrap().unit_id(), 1);
    controls
        .response_tx
        .send(rtu_frame(
            1,
            Bytes::from_static(&[0x03, 0x04, 0x00, 0x01, 0x00, 0x02]),
        ))
        .unwrap();

    assert_unexpected_length(request.await, 0x03, 2, 4);
}

#[tokio::test]
async fn register_quantity_above_limit_is_rejected_before_transport() {
    let (sink, stream, mut controls) = controlled_transport();
    let client = ModbusClient::from_transport(sink, stream, default_config());

    assert_quantity_encode_error(client.read_holding_registers(UnitId(1), 0, 126).await, 126);
    assert_quantity_encode_error(
        client.read_holding_registers_raw(UnitId(1), 0, 126).await,
        126,
    );
    assert_quantity_encode_error(client.read_input_registers(UnitId(1), 0, 126).await, 126);
    assert_quantity_encode_error(
        client
            .read_write_multiple_registers(UnitId(1), 0, 126, 0, &[0x0001])
            .await,
        126,
    );
    assert!(matches!(
        controls.sent_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn read_write_registers_accepts_maximum_read_and_write_quantities() {
    let (sink, stream, mut controls) = controlled_transport();
    let client = ModbusClient::from_transport(sink, stream, default_config());
    let write_values = vec![0x1234; 121];

    controls.outcome_tx.send(SendOutcome::Success).unwrap();
    let mut request =
        Box::pin(client.read_write_multiple_registers(UnitId(1), 0, 125, 0, &write_values));
    assert!(poll_once(request.as_mut()).await.is_none());
    let sent = controls.sent_rx.try_recv().unwrap();
    assert_eq!(sent.pdu.len(), 252);
    assert_eq!(&sent.pdu[3..5], &125u16.to_be_bytes());
    assert_eq!(&sent.pdu[7..9], &121u16.to_be_bytes());
    assert_eq!(sent.pdu[9], 242);
    let txn_id = match sent.header {
        FrameHeader::Mbap(header) => header.transaction_id.get(),
        FrameHeader::Rtu { .. } => panic!("expected MBAP request"),
    };
    let mut response_pdu = vec![0x17, 250];
    response_pdu.resize(252, 0);
    controls
        .response_tx
        .send(Frame {
            header: FrameHeader::Mbap(MbapHeader::new(txn_id, 1, 252)),
            pdu: Bytes::from(response_pdu),
        })
        .unwrap();

    assert_eq!(request.await.unwrap(), vec![0; 125]);
}

#[tokio::test]
async fn device_identification_rejects_non_advancing_continuation() {
    let addr = start_scripted_server(|fc, _req| {
        if fc == 0x2B {
            device_id_basic_response(true, 0x00, 0x00)
        } else {
            vec![fc | 0x80, 0x01]
        }
    })
    .await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    let result = client.read_device_identification(UnitId(0xFF)).await;
    assert!(
        matches!(
            result,
            Err(ClientError::InvalidDeviceIdentificationContinuation {
                previous_object_id: 0x00,
                next_object_id: 0x00,
            })
        ),
        "expected invalid continuation, got {result:?}"
    );
}

#[tokio::test]
async fn device_identification_rejects_too_many_basic_pages() {
    let addr = start_scripted_server(|fc, req| {
        if fc == 0x2B {
            let object_id = req[3];
            device_id_basic_response(true, object_id.wrapping_add(1), object_id)
        } else {
            vec![fc | 0x80, 0x01]
        }
    })
    .await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    let result = client.read_device_identification(UnitId(0xFF)).await;
    assert!(
        matches!(
            result,
            Err(ClientError::DeviceIdentificationPaginationLimit { limit: 3 })
        ),
        "expected pagination limit, got {result:?}"
    );
}
