//! Spec V1.1b3 §§6.1–6.4 and §6.17: client success responses match the request shape.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus_client::{ClientConfig, ClientError, ModbusClient};
use rusty_modbus_codec::{DecodeError, decode_response};
use rusty_modbus_frame::OwnedResponsePdu;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_tcp::TransportError;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_types::{MbapHeader, UnitId};
use tokio::sync::mpsc;

struct TestSink {
    sent: mpsc::UnboundedSender<Frame>,
}

impl TransportSink for TestSink {
    async fn send(&mut self, frame: Frame) -> Result<(), TransportError> {
        self.sent
            .send(frame)
            .map_err(|_| TransportError::Disconnected)
    }
}

struct TestStream {
    responses: mpsc::UnboundedReceiver<Frame>,
}

impl TransportStream for TestStream {
    async fn recv(&mut self) -> Result<Frame, TransportError> {
        self.responses
            .recv()
            .await
            .ok_or(TransportError::Disconnected)
    }
}

struct TestControls {
    sent: mpsc::UnboundedReceiver<Frame>,
    responses: mpsc::UnboundedSender<Frame>,
}

fn test_transport() -> (TestSink, TestStream, TestControls) {
    let (sent_tx, sent_rx) = mpsc::unbounded_channel();
    let (response_tx, response_rx) = mpsc::unbounded_channel();
    (
        TestSink { sent: sent_tx },
        TestStream {
            responses: response_rx,
        },
        TestControls {
            sent: sent_rx,
            responses: response_tx,
        },
    )
}

fn config() -> ClientConfig {
    ClientConfig {
        timeout: Duration::from_secs(2),
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

fn mbap_response(request: Frame, pdu: &'static [u8]) -> Frame {
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
async fn tcp_typed_client_enforces_exact_length_padding_and_exception_shape() {
    let (sink, stream, mut controls) = test_transport();
    let client = ModbusClient::from_transport(sink, stream, config());

    let mut registers = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
    assert!(poll_once(registers.as_mut()).await.is_none());
    let sent = controls.sent.try_recv().unwrap();
    controls
        .responses
        .send(mbap_response(sent, &[0x03, 0x04, 0x00, 0x01, 0x00, 0x02]))
        .unwrap();
    assert!(matches!(
        registers.await,
        Err(ClientError::UnexpectedResponseLength {
            function_code: 0x03,
            expected: 2,
            actual: 4,
        })
    ));

    let mut coils = Box::pin(client.read_coils(UnitId(1), 0, 9));
    assert!(poll_once(coils.as_mut()).await.is_none());
    let sent = controls.sent.try_recv().unwrap();
    controls
        .responses
        .send(mbap_response(sent, &[0x01, 0x02, 0xFF, 0x80]))
        .unwrap();
    assert!(matches!(
        coils.await,
        Err(ClientError::UnexpectedResponsePadding {
            function_code: 0x01,
            invalid_mask: 0xFE,
            actual: 0x80,
        })
    ));

    let mut exception = Box::pin(client.read_input_registers(UnitId(1), 0, 125));
    assert!(poll_once(exception.as_mut()).await.is_none());
    let sent = controls.sent.try_recv().unwrap();
    controls
        .responses
        .send(mbap_response(sent, &[0x84, 0x02]))
        .unwrap();
    assert!(matches!(exception.await, Err(ClientError::Exception(_))));
}

#[tokio::test]
async fn rtu_typed_client_uses_read_quantity_for_fc17_shape() {
    let (sink, stream, mut controls) = test_transport();
    let client = ModbusClient::from_rtu_transport(sink, stream, config());

    let mut request =
        Box::pin(client.read_write_multiple_registers(UnitId(7), 0, 1, 0, &[0x0001, 0x0002]));
    assert!(poll_once(request.as_mut()).await.is_none());
    assert_eq!(controls.sent.try_recv().unwrap().unit_id(), 7);
    controls
        .responses
        .send(Frame {
            header: FrameHeader::Rtu { unit_id: 7 },
            pdu: Bytes::from_static(&[0x17, 0x04, 0x00, 0x01, 0x00, 0x02]),
        })
        .unwrap();

    assert!(matches!(
        request.await,
        Err(ClientError::UnexpectedResponseLength {
            function_code: 0x17,
            expected: 2,
            actual: 4,
        })
    ));
}

#[test]
fn register_response_decoders_reject_odd_data_lengths() {
    for pdu in [
        &[0x03, 0x03, 0x00, 0x01, 0x02][..],
        &[0x04, 0x03, 0x00, 0x01, 0x02][..],
        &[0x17, 0x03, 0x00, 0x01, 0x02][..],
    ] {
        assert!(matches!(
            decode_response(pdu),
            Err(DecodeError::InvalidRegisterDataLength { length: 3 })
        ));
        assert!(matches!(
            OwnedResponsePdu::from_pdu(Bytes::copy_from_slice(pdu)),
            Err(DecodeError::InvalidRegisterDataLength { length: 3 })
        ));
    }
}
