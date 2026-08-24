//! Integration tests for ModbusClient.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus_client::{ClientConfig, ClientError, ModbusClient, RetryConfig};
use rusty_modbus_codec::EncodeError;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_tcp::TransportError;
use rusty_modbus_tcp::config::TcpServerConfig;
use rusty_modbus_tcp::listener::TcpServerListener;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_types::{MbapHeader, UnitId};
use tokio::sync::mpsc;

enum SendOutcome {
    Success,
    Failure,
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
            None => Err(TransportError::Disconnected),
        }
    }
}

struct ControlledStream {
    response_rx: mpsc::UnboundedReceiver<Frame>,
}

impl TransportStream for ControlledStream {
    async fn recv(&mut self) -> Result<Frame, TransportError> {
        self.response_rx
            .recv()
            .await
            .ok_or(TransportError::Disconnected)
    }
}

struct TransportControls {
    sent_rx: mpsc::UnboundedReceiver<Frame>,
    outcome_tx: mpsc::UnboundedSender<SendOutcome>,
    response_tx: mpsc::UnboundedSender<Frame>,
}

fn controlled_transport() -> (ControlledSink, ControlledStream, TransportControls) {
    let (sent_tx, sent_rx) = mpsc::unbounded_channel();
    let (outcome_tx, outcome_rx) = mpsc::unbounded_channel();
    let (response_tx, response_rx) = mpsc::unbounded_channel();

    (
        ControlledSink {
            sent_tx,
            outcome_rx,
        },
        ControlledStream { response_rx },
        TransportControls {
            sent_rx,
            outcome_tx,
            response_tx,
        },
    )
}

fn rtu_frame(unit_id: u8, pdu: impl Into<Bytes>) -> Frame {
    Frame {
        header: FrameHeader::Rtu { unit_id },
        pdu: pdu.into(),
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

#[tokio::test]
async fn shutdown_cancels_pending() {
    let addr = start_register_server().await;
    let client = std::sync::Arc::new(ModbusClient::connect(addr, default_config()).await.unwrap());

    client.shutdown().await;

    let result = client.read_holding_registers(UnitId(0xFF), 0, 1).await;
    assert!(matches!(result, Err(ClientError::NotConnected)));
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
