//! TCP client connection establishment.

use std::io::ErrorKind;
use std::net::SocketAddr;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use rusty_modbus_frame::frame::Frame;
use rusty_modbus_frame::mbap::MbapCodec;
use tokio::io::ReadBuf;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_util::codec::Framed;
use tracing::{debug, trace, warn};

use crate::config::TcpConfig;
use crate::error::TransportError;
use crate::transport::{TransportConnect, TransportSink, TransportStream};

/// Interval between TCP keepalive probes once the idle `keepalive` time
/// elapses. Bounds dead-peer detection latency instead of inheriting the long
/// OS default probe interval.
const KEEPALIVE_PROBE_INTERVAL: Duration = Duration::from_secs(10);

/// TCP client transport — connects to a Modbus/TCP server.
pub struct TcpTransport;

impl TransportConnect for TcpTransport {
    type Sink = TcpSink;
    type Stream = TcpRecvStream;

    async fn connect(
        config: TcpConfig,
        addr: SocketAddr,
    ) -> Result<(Self::Sink, Self::Stream), TransportError> {
        debug!(
            addr = %addr,
            connect_timeout = ?config.connect_timeout,
            "connecting TCP transport"
        );
        let stream = timeout(config.connect_timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| TransportError::Timeout)?
            .map_err(TransportError::Io)?;

        configure_socket(&stream, &config)?;
        debug!(addr = %addr, "TCP transport connected");

        let framed = Framed::new(stream, MbapCodec);
        let (sink, recv_stream) = framed.split();

        Ok((
            TcpSink::new(sink, config.write_timeout),
            TcpRecvStream::new(recv_stream, config.read_timeout),
        ))
    }
}

/// Apply TCP socket options per Modbus TCP Guide §4.2–4.3.
fn configure_socket(stream: &TcpStream, config: &TcpConfig) -> Result<(), TransportError> {
    stream.set_nodelay(config.tcp_nodelay)?;
    trace!(tcp_nodelay = config.tcp_nodelay, "configured TCP nodelay");

    let sock_ref = socket2::SockRef::from(stream);
    if let Some(keepalive_duration) = config.keepalive {
        // Set an explicit probe interval as well as the idle time so that
        // dead-peer detection has a bounded, predictable upper bound rather
        // than relying on the (long) OS default probe interval. The reader no
        // longer tears down idle connections on read-timeout, so keepalive is
        // the mechanism that eventually surfaces a silently half-open socket.
        let keepalive = socket2::TcpKeepalive::new()
            .with_time(keepalive_duration)
            .with_interval(KEEPALIVE_PROBE_INTERVAL);
        sock_ref.set_tcp_keepalive(&keepalive)?;
        trace!(
            keepalive = ?keepalive_duration,
            interval = ?KEEPALIVE_PROBE_INTERVAL,
            "configured TCP keepalive"
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Split halves
// ---------------------------------------------------------------------------

type InnerSink = SplitSink<Framed<TcpStream, MbapCodec>, Frame>;
type InnerStream = SplitStream<Framed<TcpStream, MbapCodec>>;

/// An instantaneous passive observation of an idle TCP transport pair.
///
/// This classification reports only decoder-buffered bytes and the socket state
/// observable by one non-consuming, non-waiting `peek`. In particular,
/// [`Self::NoAdverseSignal`] is not a liveness guarantee, a protocol
/// synchronization guarantee, or a promise that input will not arrive after the
/// observation.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpIdleObservation {
    /// No decoder-buffered bytes or immediately observable adverse socket state.
    NoAdverseSignal,
    /// Input is buffered by the decoder or immediately readable from the socket.
    QueuedInput,
    /// A non-consuming socket peek observed peer EOF.
    PeerClosed,
    /// A non-consuming socket peek returned a bounded I/O error kind.
    SocketError(ErrorKind),
    /// The supplied sink and receive stream did not originate from the same split.
    MismatchedHalves,
}

/// Passively inspect and reconstruct an idle TCP transport pair.
///
/// The returned tuple contains the original sink, receive stream, and their
/// instantaneous observation, in that order. For matching halves this preserves
/// the codec, decoder and encoder buffers, socket, and read/write timeout
/// settings. Mismatched halves are returned unchanged with
/// [`TcpIdleObservation::MismatchedHalves`], allowing their actual matching
/// counterparts to reconstruct them later.
///
/// This operation performs no write, consumes no receive bytes, does not await
/// readiness, and does not actively probe the peer. A no-adverse-signal result
/// therefore cannot establish peer liveness or protocol synchronization.
#[must_use]
pub fn inspect_idle_tcp(
    sink: TcpSink,
    stream: TcpRecvStream,
) -> (TcpSink, TcpRecvStream, TcpIdleObservation) {
    let TcpSink {
        inner: sink,
        write_timeout,
    } = sink;
    let TcpRecvStream {
        inner: stream,
        read_timeout,
    } = stream;

    match sink.reunite(stream) {
        Ok(framed) => {
            let observation = observe_idle_framed(&framed);
            let (sink, stream) = framed.split();
            (
                TcpSink::new(sink, write_timeout),
                TcpRecvStream::new(stream, read_timeout),
                observation,
            )
        }
        Err(error) => (
            TcpSink::new(error.0, write_timeout),
            TcpRecvStream::new(error.1, read_timeout),
            TcpIdleObservation::MismatchedHalves,
        ),
    }
}

fn observe_idle_framed(framed: &Framed<TcpStream, MbapCodec>) -> TcpIdleObservation {
    if !framed.read_buffer().is_empty() {
        return TcpIdleObservation::QueuedInput;
    }

    let mut byte = [0_u8; 1];
    let mut buffer = ReadBuf::new(&mut byte);
    let mut context = Context::from_waker(Waker::noop());
    let result = framed.get_ref().poll_peek(&mut context, &mut buffer);
    classify_peek(result)
}

fn classify_peek(result: Poll<std::io::Result<usize>>) -> TcpIdleObservation {
    match result {
        Poll::Pending => TcpIdleObservation::NoAdverseSignal,
        Poll::Ready(Ok(0)) => TcpIdleObservation::PeerClosed,
        Poll::Ready(Ok(_)) => TcpIdleObservation::QueuedInput,
        Poll::Ready(Err(error)) => TcpIdleObservation::SocketError(error.kind()),
    }
}

/// Write half of a TCP transport.
pub struct TcpSink {
    inner: InnerSink,
    write_timeout: Option<Duration>,
}

impl TcpSink {
    /// Create from a framed sink half.
    pub(crate) fn new(sink: InnerSink, write_timeout: Option<Duration>) -> Self {
        Self {
            inner: sink,
            write_timeout,
        }
    }
}

impl TransportSink for TcpSink {
    async fn send(&mut self, frame: Frame) -> Result<(), TransportError> {
        let unit_id = frame.unit_id();
        let pdu_len = frame.pdu.len();
        trace!(unit_id, pdu_len, "sending TCP Modbus frame");
        let fut = SinkExt::send(&mut self.inner, frame);
        let result = if let Some(dur) = self.write_timeout {
            match timeout(dur, fut).await {
                Ok(result) => result.map_err(TransportError::Frame),
                Err(_) => Err(TransportError::Timeout),
            }
        } else {
            fut.await.map_err(TransportError::Frame)
        };
        if let Err(error) = &result {
            warn!(unit_id, pdu_len, error = %error, "failed to send TCP Modbus frame");
        }
        result
    }
}

/// Read half of a TCP transport.
pub struct TcpRecvStream {
    inner: InnerStream,
    read_timeout: Option<Duration>,
}

impl TcpRecvStream {
    /// Create from a framed stream half.
    pub(crate) fn new(stream: InnerStream, read_timeout: Option<Duration>) -> Self {
        Self {
            inner: stream,
            read_timeout,
        }
    }
}

impl TransportStream for TcpRecvStream {
    async fn recv(&mut self) -> Result<Frame, TransportError> {
        let fut = self.inner.next();
        let item = if let Some(dur) = self.read_timeout {
            if let Ok(item) = timeout(dur, fut).await {
                item
            } else {
                trace!(timeout = ?dur, "timed out waiting for TCP Modbus frame");
                return Err(TransportError::Timeout);
            }
        } else {
            fut.await
        };

        match item {
            Some(Ok(frame)) => {
                trace!(
                    unit_id = frame.unit_id(),
                    pdu_len = frame.pdu.len(),
                    "received TCP Modbus frame"
                );
                Ok(frame)
            }
            Some(Err(e)) => {
                warn!(error = %e, "failed to decode TCP Modbus frame");
                Err(TransportError::Frame(e))
            }
            None => {
                debug!("TCP Modbus stream disconnected");
                Err(TransportError::Disconnected)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, ErrorKind};

    use bytes::Bytes;
    use rusty_modbus_frame::frame::FrameHeader;
    use rusty_modbus_types::MbapHeader;
    use tokio::io::AsyncWriteExt;

    use super::*;

    const TEST_ADU: [u8; 12] = [
        0x12, 0x34, // transaction ID
        0x00, 0x00, // protocol ID
        0x00, 0x06, // unit ID plus five PDU bytes
        0xff, // unit ID
        0x03, 0x00, 0x00, 0x00, 0x01, // PDU
    ];

    async fn connected_pair(
        config: TcpConfig,
    ) -> ((TcpSink, TcpRecvStream), tokio::net::TcpStream) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (client, accepted) =
            tokio::join!(TcpTransport::connect(config, addr), listener.accept());
        (client.unwrap(), accepted.unwrap().0)
    }

    async fn wait_for_observation(
        mut sink: TcpSink,
        mut stream: TcpRecvStream,
        expected: TcpIdleObservation,
    ) -> (TcpSink, TcpRecvStream) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let (next_sink, next_stream, observation) = inspect_idle_tcp(sink, stream);
                sink = next_sink;
                stream = next_stream;
                if observation == expected {
                    return (sink, stream);
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("socket state should become immediately observable")
    }

    #[tokio::test]
    async fn clean_matching_pair_is_preserved_without_waiting() {
        let read_timeout = Some(Duration::from_millis(17));
        let write_timeout = Some(Duration::from_millis(23));
        let ((sink, stream), _peer) = connected_pair(TcpConfig {
            read_timeout,
            write_timeout,
            ..TcpConfig::default()
        })
        .await;

        let (sink, stream, observation) = inspect_idle_tcp(sink, stream);

        assert_eq!(observation, TcpIdleObservation::NoAdverseSignal);
        assert_eq!(sink.write_timeout, write_timeout);
        assert_eq!(stream.read_timeout, read_timeout);
        let (_, _, second_observation) = inspect_idle_tcp(sink, stream);
        assert_eq!(second_observation, TcpIdleObservation::NoAdverseSignal);
    }

    #[tokio::test]
    async fn complete_queued_input_is_reported_without_consuming_it() {
        let ((sink, stream), peer) = connected_pair(TcpConfig::default()).await;
        let mut peer = Framed::new(peer, MbapCodec);
        let frame = Frame {
            header: FrameHeader::Mbap(MbapHeader::new(0x1234, 0xff, 5)),
            pdu: Bytes::from_static(&[0x03, 0x00, 0x00, 0x00, 0x01]),
        };
        peer.send(frame).await.unwrap();

        let (sink, stream) =
            wait_for_observation(sink, stream, TcpIdleObservation::QueuedInput).await;
        let (sink, stream, repeated) = inspect_idle_tcp(sink, stream);
        assert_eq!(repeated, TcpIdleObservation::QueuedInput);

        let mut stream = stream;
        let received = stream.recv().await.unwrap();
        assert!(matches!(
            received.header,
            FrameHeader::Mbap(header) if header.transaction_id.get() == 0x1234
        ));
        drop(sink);
    }

    #[tokio::test]
    async fn partial_decoder_buffer_is_reported_and_preserved() {
        let ((sink, mut stream), mut peer) = connected_pair(TcpConfig {
            read_timeout: Some(Duration::from_millis(20)),
            ..TcpConfig::default()
        })
        .await;
        peer.write_all(&TEST_ADU[..3]).await.unwrap();

        assert!(matches!(stream.recv().await, Err(TransportError::Timeout)));
        let (sink, stream, first) = inspect_idle_tcp(sink, stream);
        assert_eq!(first, TcpIdleObservation::QueuedInput);
        let (_sink, mut stream, second) = inspect_idle_tcp(sink, stream);
        assert_eq!(second, TcpIdleObservation::QueuedInput);

        peer.write_all(&TEST_ADU[3..]).await.unwrap();
        let received = stream.recv().await.unwrap();
        assert!(matches!(
            received.header,
            FrameHeader::Mbap(header) if header.transaction_id.get() == 0x1234
        ));
    }

    #[tokio::test]
    async fn peer_eof_is_reported_without_consuming() {
        let ((sink, stream), peer) = connected_pair(TcpConfig::default()).await;
        drop(peer);

        let (sink, stream) =
            wait_for_observation(sink, stream, TcpIdleObservation::PeerClosed).await;
        let (_, _, repeated) = inspect_idle_tcp(sink, stream);
        assert_eq!(repeated, TcpIdleObservation::PeerClosed);
    }

    #[tokio::test]
    async fn mismatched_halves_are_returned_for_their_actual_pairs() {
        let ((sink_a, stream_a), _peer_a) = connected_pair(TcpConfig::default()).await;
        let ((sink_b, stream_b), _peer_b) = connected_pair(TcpConfig::default()).await;

        let (sink_a, stream_b, observation) = inspect_idle_tcp(sink_a, stream_b);
        assert_eq!(observation, TcpIdleObservation::MismatchedHalves);

        let (_, _, observation_a) = inspect_idle_tcp(sink_a, stream_a);
        let (_, _, observation_b) = inspect_idle_tcp(sink_b, stream_b);
        assert_eq!(observation_a, TcpIdleObservation::NoAdverseSignal);
        assert_eq!(observation_b, TcpIdleObservation::NoAdverseSignal);
    }

    #[test]
    fn socket_error_is_reduced_to_its_bounded_kind() {
        let observation = classify_peek(Poll::Ready(Err(io::Error::from(
            ErrorKind::ConnectionReset,
        ))));

        assert_eq!(
            observation,
            TcpIdleObservation::SocketError(ErrorKind::ConnectionReset)
        );
    }
}
