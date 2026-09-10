//! Modbus/TCP Guide V1.0b §4.4.1 / TCP-006: live ID and reclaim evidence.
//!
//! Controlled transport frames exercise the public client, not a 65k network wrap
//! campaign. Counter-seeded wrap/collision cases live in client transaction unit
//! tests. Ancient replies with the same full ID after an entire reuse cycle remain
//! wire-ambiguous. This file does not prove gateway, TLS, pool or physical RTU behavior.

#![forbid(unsafe_code)]

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus_client::{
    ClientConfig, ClientError, ModbusClient, RetryConfig, SessionRetirementReason,
    SessionReuseVerdict,
};
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_tcp::TransportError;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_types::{MbapHeader, UnitId};
use tokio::sync::mpsc;

struct ReclaimSink {
    sent: mpsc::Sender<Frame>,
    outcomes: mpsc::Receiver<Result<(), TransportError>>,
}

impl TransportSink for ReclaimSink {
    async fn send(&mut self, frame: Frame) -> Result<(), TransportError> {
        self.sent
            .send(frame)
            .await
            .map_err(|_| TransportError::Disconnected)?;
        self.outcomes
            .recv()
            .await
            .unwrap_or(Err(TransportError::Disconnected))
    }
}

struct ReclaimStream {
    responses: mpsc::Receiver<Frame>,
}

impl TransportStream for ReclaimStream {
    async fn recv(&mut self) -> Result<Frame, TransportError> {
        self.responses
            .recv()
            .await
            .ok_or(TransportError::Disconnected)
    }
}

struct Controls {
    sent: mpsc::Receiver<Frame>,
    outcomes: mpsc::Sender<Result<(), TransportError>>,
    responses: mpsc::Sender<Frame>,
}

fn transport() -> (ReclaimSink, ReclaimStream, Controls) {
    let (sent_tx, sent) = mpsc::channel(32);
    let (outcomes, outcomes_rx) = mpsc::channel(32);
    let (responses, response_rx) = mpsc::channel(32);
    (
        ReclaimSink {
            sent: sent_tx,
            outcomes: outcomes_rx,
        },
        ReclaimStream {
            responses: response_rx,
        },
        Controls {
            sent,
            outcomes,
            responses,
        },
    )
}

fn config() -> ClientConfig {
    ClientConfig {
        // Watchdogs below expire before real request timers. Correctness is driven
        // by explicit channel events; this target needs no Tokio test-util feature.
        timeout: Duration::from_secs(60),
        shutdown_timeout: Duration::from_secs(1),
        max_in_flight: 1,
        retry: RetryConfig {
            max_retries: 0,
            retry_delay: Duration::ZERO,
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

fn transaction_id(frame: &Frame) -> u16 {
    let FrameHeader::Mbap(header) = frame.header else {
        panic!("expected a Modbus/TCP request");
    };
    header.transaction_id.get()
}

fn response(request: &Frame, value: u16) -> Frame {
    let [high, low] = value.to_be_bytes();
    Frame {
        header: FrameHeader::Mbap(MbapHeader::new(
            transaction_id(request),
            request.unit_id(),
            4,
        )),
        pdu: Bytes::copy_from_slice(&[0x03, 0x02, high, low]),
    }
}

#[tokio::test]
async fn spec_tcp006_cancelled_request_stale_reply_cannot_complete_replacement() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (sink, stream, mut controls) = transport();
        let client = ModbusClient::from_transport(sink, stream, config());
        let mut first = None;
        for _ in 0..16 {
            controls.outcomes.try_send(Ok(())).unwrap();
            let mut request = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
            assert!(poll_once(request.as_mut()).await.is_none());
            first.get_or_insert(controls.sent.try_recv().unwrap());
            drop(request);
        }
        controls.outcomes.try_send(Ok(())).unwrap();
        let mut next = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
        assert!(poll_once(next.as_mut()).await.is_none());
        let sent = controls.sent.try_recv().unwrap();
        let first = first.unwrap();
        assert_ne!(transaction_id(&first), transaction_id(&sent));
        assert_eq!(transaction_id(&first) % 16, transaction_id(&sent) % 16);
        // FIFO makes the wrong old payload arrive before the correct payload.
        // No pending assertion is used as a proxy for reader consumption.
        controls
            .responses
            .try_send(response(&first, 0xDEAD))
            .unwrap();
        controls
            .responses
            .try_send(response(&sent, 0xBEEF))
            .unwrap();
        assert_eq!(next.await.unwrap(), vec![0xBEEF]);
        assert!(controls.sent.try_recv().is_err());
        client.shutdown().await;
        assert_eq!(
            client.session_reuse_verdict(),
            SessionReuseVerdict::Retire(SessionRetirementReason::DispatchCancelled)
        );
    })
    .await
    .expect("controlled cancellation/reclaim sequence must finish");
}

#[tokio::test]
async fn spec_tcp006_transport_timeout_retry_ignores_old_attempt_payload() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (sink, stream, mut controls) = transport();
        let mut config = config();
        config.retry.max_retries = 1;
        let client = ModbusClient::from_transport(sink, stream, config);
        // Explicit transport timeout, not a slept wall-clock assertion. The client
        // integration test separately exercises exact paused response deadlines.
        controls
            .outcomes
            .try_send(Err(TransportError::Timeout))
            .unwrap();
        let mut request = Box::pin(client.read_holding_registers(UnitId(1), 0, 1));
        assert!(poll_once(request.as_mut()).await.is_none());
        let first = controls.sent.try_recv().unwrap();
        let peer = async {
            let retry = controls.sent.recv().await.unwrap();
            assert_ne!(transaction_id(&retry), transaction_id(&first));
            assert_ne!(transaction_id(&retry), 0);
            assert_eq!(retry.pdu, first.pdu);
            controls.outcomes.send(Ok(())).await.unwrap();
            controls
                .responses
                .send(response(&first, 0xDEAD))
                .await
                .unwrap();
            controls
                .responses
                .send(response(&retry, 0xCAFE))
                .await
                .unwrap();
        };
        let (result, ()) = tokio::join!(request, peer);
        assert_eq!(result.unwrap(), vec![0xCAFE]);
        assert!(controls.sent.try_recv().is_err());
        client.shutdown().await;
        assert_eq!(
            client.session_reuse_verdict(),
            SessionReuseVerdict::Retire(SessionRetirementReason::RequestTimedOut)
        );
    })
    .await
    .expect("controlled retry/reclaim sequence must finish");
}

#[tokio::test]
async fn spec_tcp006_full_capacity_reclaim_keeps_other_live_transactions() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (sink, stream, mut controls) = transport();
        let client = ModbusClient::from_transport(
            sink,
            stream,
            ClientConfig {
                max_in_flight: 17, // Public construction clamps this to the project's 16-slot cap.
                ..config()
            },
        );
        let mut pending = Vec::new();
        let mut ids = Vec::new();
        for value in 0..16_u16 {
            controls.outcomes.try_send(Ok(())).unwrap();
            let mut request = Box::pin(client.read_holding_registers(UnitId(1), value, 1));
            assert!(poll_once(request.as_mut()).await.is_none());
            let sent = controls.sent.try_recv().unwrap();
            assert_ne!(transaction_id(&sent), 0);
            assert!(!ids.contains(&transaction_id(&sent)));
            ids.push(transaction_id(&sent));
            pending.push((request, sent, 0x1000 + value));
        }
        // Public admission waits at capacity. TransactionConflict itself is covered
        // at the private manager seam, not fabricated as a public overload response.
        let mut replacement = Box::pin(client.read_holding_registers(UnitId(1), 20, 1));
        assert!(poll_once(replacement.as_mut()).await.is_none());
        assert!(controls.sent.try_recv().is_err());
        for (request, _, _) in &mut pending {
            assert!(poll_once(request.as_mut()).await.is_none());
        }
        let (cancelled, stale, _) = pending.remove(0);
        drop(cancelled);
        controls.outcomes.try_send(Ok(())).unwrap();
        assert!(poll_once(replacement.as_mut()).await.is_none());
        let fresh = controls.sent.try_recv().unwrap();
        assert_ne!(transaction_id(&fresh), transaction_id(&stale));
        assert_eq!(transaction_id(&fresh) % 16, transaction_id(&stale) % 16);
        assert!(
            pending
                .iter()
                .all(|(_, frame, _)| transaction_id(frame) != transaction_id(&fresh))
        );
        controls
            .responses
            .try_send(response(&stale, 0xDEAD))
            .unwrap();
        controls
            .responses
            .try_send(response(&fresh, 0xBEEF))
            .unwrap();
        for (_, frame, value) in pending.iter().rev() {
            controls
                .responses
                .try_send(response(frame, *value))
                .unwrap();
        }
        assert_eq!(replacement.await.unwrap(), vec![0xBEEF]);
        for (request, _, value) in pending {
            assert_eq!(request.await.unwrap(), vec![value]);
        }
        client.shutdown().await;
        assert!(matches!(
            client.read_holding_registers(UnitId(1), 0, 1).await,
            Err(ClientError::NotConnected)
        ));
        assert_eq!(
            client.session_reuse_verdict(),
            SessionReuseVerdict::Retire(SessionRetirementReason::DispatchCancelled)
        );
    })
    .await
    .expect("controlled capacity/reclaim sequence must finish");
}
