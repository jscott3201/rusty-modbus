//! TCP client connection establishment.

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use rusty_modbus_frame::frame::Frame;
use rusty_modbus_frame::mbap::MbapCodec;
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
