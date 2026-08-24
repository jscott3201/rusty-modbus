//! Spec V1.1b3 §7 and client safety policy: retries preserve request semantics.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus_client::{ClientConfig, ClientError, ModbusClient, RetryConfig};
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_tcp::TransportError;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_types::{ExceptionCode, MbapHeader, UnitId};
use tokio::sync::mpsc;

enum SendOutcome {
    Success,
    Timeout,
}

struct PolicySink {
    sent: mpsc::UnboundedSender<Frame>,
    outcomes: mpsc::UnboundedReceiver<SendOutcome>,
}

impl TransportSink for PolicySink {
    async fn send(&mut self, frame: Frame) -> Result<(), TransportError> {
        self.sent
            .send(frame)
            .map_err(|_| TransportError::Disconnected)?;
        match self.outcomes.recv().await {
            Some(SendOutcome::Success) => Ok(()),
            Some(SendOutcome::Timeout) => Err(TransportError::Timeout),
            None => Err(TransportError::Disconnected),
        }
    }
}

struct PolicyStream {
    responses: mpsc::UnboundedReceiver<Frame>,
}

impl TransportStream for PolicyStream {
    async fn recv(&mut self) -> Result<Frame, TransportError> {
        self.responses
            .recv()
            .await
            .ok_or(TransportError::Disconnected)
    }
}

struct PolicyControls {
    sent: mpsc::UnboundedReceiver<Frame>,
    outcomes: mpsc::UnboundedSender<SendOutcome>,
    responses: mpsc::UnboundedSender<Frame>,
}

fn policy_transport() -> (PolicySink, PolicyStream, PolicyControls) {
    let (sent_tx, sent_rx) = mpsc::unbounded_channel();
    let (outcome_tx, outcome_rx) = mpsc::unbounded_channel();
    let (response_tx, response_rx) = mpsc::unbounded_channel();
    (
        PolicySink {
            sent: sent_tx,
            outcomes: outcome_rx,
        },
        PolicyStream {
            responses: response_rx,
        },
        PolicyControls {
            sent: sent_rx,
            outcomes: outcome_tx,
            responses: response_tx,
        },
    )
}

fn config() -> ClientConfig {
    ClientConfig {
        timeout: Duration::from_millis(20),
        retry: RetryConfig {
            max_retries: 1,
            retry_delay: Duration::from_millis(1),
            ..RetryConfig::default()
        },
        ..ClientConfig::default()
    }
}

async fn poll_once<F: Future>(future: Pin<&mut F>) -> Option<F::Output> {
    tokio::select! {
        biased;
        output = future => Some(output),
        () = std::future::ready(()) => None,
    }
}

fn mbap_response(request: &Frame, pdu: &'static [u8]) -> Frame {
    let FrameHeader::Mbap(header) = request.header else {
        panic!("expected MBAP request header");
    };
    Frame {
        header: FrameHeader::Mbap(MbapHeader::new(
            header.transaction_id.get(),
            request.unit_id(),
            pdu.len() as u16,
        )),
        pdu: Bytes::from_static(pdu),
    }
}

#[tokio::test]
async fn acknowledge_is_terminal_even_when_selected_for_retry() {
    let (sink, stream, mut controls) = policy_transport();
    let mut client_config = config();
    client_config.retry.retryable_exceptions = vec![ExceptionCode::Acknowledge];
    let client = ModbusClient::from_transport(sink, stream, client_config);

    controls.outcomes.send(SendOutcome::Success).unwrap();
    let mut request = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(request.as_mut()).await.is_none());
    let sent = controls.sent.try_recv().unwrap();
    controls
        .responses
        .send(mbap_response(&sent, &[0x83, 0x05]))
        .unwrap();

    assert!(matches!(
        request.await,
        Err(ClientError::Exception(exception))
            if exception.exception_code == ExceptionCode::Acknowledge
    ));
    assert!(controls.sent.try_recv().is_err());
}

#[tokio::test]
async fn typed_tcp_and_rtu_writes_do_not_replay_response_timeout() {
    let (tcp_sink, tcp_stream, mut tcp_controls) = policy_transport();
    let tcp_client = ModbusClient::from_transport(tcp_sink, tcp_stream, config());
    tcp_controls.outcomes.send(SendOutcome::Success).unwrap();

    assert!(matches!(
        tcp_client.write_single_register(UnitId(1), 0, 0x1234).await,
        Err(ClientError::Timeout)
    ));
    assert_eq!(tcp_controls.sent.try_recv().unwrap().pdu[0], 0x06);
    assert!(tcp_controls.sent.try_recv().is_err());

    let (rtu_sink, rtu_stream, mut rtu_controls) = policy_transport();
    let rtu_client = ModbusClient::from_rtu_transport(rtu_sink, rtu_stream, config());
    rtu_controls.outcomes.send(SendOutcome::Success).unwrap();

    assert!(matches!(
        rtu_client
            .read_write_multiple_registers(UnitId(1), 0, 1, 0, &[0x1234])
            .await,
        Err(ClientError::Timeout)
    ));
    assert_eq!(rtu_controls.sent.try_recv().unwrap().pdu[0], 0x17);
    assert!(rtu_controls.sent.try_recv().is_err());
}

#[tokio::test]
async fn transport_timeout_retries_read_and_remains_bounded() {
    let (sink, stream, mut controls) = policy_transport();
    let client = ModbusClient::from_transport(sink, stream, config());
    controls.outcomes.send(SendOutcome::Timeout).unwrap();
    controls.outcomes.send(SendOutcome::Success).unwrap();

    let mut request = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(request.as_mut()).await.is_none());
    assert_eq!(controls.sent.try_recv().unwrap().pdu[0], 0x03);
    tokio::time::sleep(Duration::from_millis(2)).await;
    assert!(poll_once(request.as_mut()).await.is_none());
    let retry = controls.sent.try_recv().unwrap();
    controls
        .responses
        .send(mbap_response(&retry, &[0x03, 0x02, 0x00, 0x2A]))
        .unwrap();

    assert_eq!(request.await.unwrap(), vec![0x002A]);
    assert!(controls.sent.try_recv().is_err());
}
