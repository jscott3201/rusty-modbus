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

use crate::config::TcpConfig;
use crate::error::TransportError;
use crate::transport::{TransportConnect, TransportSink, TransportStream};

/// TCP client transport — connects to a Modbus/TCP server.
pub struct TcpTransport;

impl TransportConnect for TcpTransport {
    type Sink = TcpSink;
    type Stream = TcpRecvStream;

    async fn connect(
        config: TcpConfig,
        addr: SocketAddr,
    ) -> Result<(Self::Sink, Self::Stream), TransportError> {
        let stream = timeout(config.connect_timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| TransportError::Timeout)?
            .map_err(TransportError::Io)?;

        configure_socket(&stream, &config)?;

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

    let sock_ref = socket2::SockRef::from(stream);
    if let Some(keepalive_duration) = config.keepalive {
        let keepalive = socket2::TcpKeepalive::new().with_time(keepalive_duration);
        sock_ref.set_tcp_keepalive(&keepalive)?;
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
        let fut = SinkExt::send(&mut self.inner, frame);
        if let Some(dur) = self.write_timeout {
            timeout(dur, fut)
                .await
                .map_err(|_| TransportError::Timeout)?
                .map_err(TransportError::Frame)
        } else {
            fut.await.map_err(TransportError::Frame)
        }
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
            timeout(dur, fut)
                .await
                .map_err(|_| TransportError::Timeout)?
        } else {
            fut.await
        };

        match item {
            Some(Ok(frame)) => Ok(frame),
            Some(Err(e)) => Err(TransportError::Frame(e)),
            None => Err(TransportError::Disconnected),
        }
    }
}
