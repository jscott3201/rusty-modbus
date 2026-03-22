//! RTU-over-TCP transport.
//!
//! Carries RTU-framed Modbus PDUs over a TCP connection instead of a physical
//! serial line. This is useful for remote serial bridges and for testing
//! without serial hardware.

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use modbus_frame::frame::Frame;
use modbus_frame::rtu_tcp::RtuOverTcpCodec;
use modbus_tcp::transport::{TransportSink, TransportStream};
use modbus_tcp::{TcpConfig, TransportError};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_util::codec::Framed;

use crate::error::RtuError;

type InnerSink = SplitSink<Framed<TcpStream, RtuOverTcpCodec>, Frame>;
type InnerStream = SplitStream<Framed<TcpStream, RtuOverTcpCodec>>;

/// RTU-over-TCP transport factory.
///
/// Establishes a TCP connection and frames data using the RTU wire format
/// (unit-id + PDU + CRC-16) rather than MBAP.
pub struct RtuOverTcpTransport;

impl RtuOverTcpTransport {
    /// Connect to a remote RTU-over-TCP endpoint and return split halves.
    ///
    /// # Errors
    ///
    /// Returns [`RtuError::Timeout`] if the connection cannot be established
    /// within `config.connect_timeout`, or [`RtuError::Io`] on socket errors.
    pub async fn connect(
        addr: SocketAddr,
        config: TcpConfig,
    ) -> Result<(RtuTcpSink, RtuTcpRecvStream), RtuError> {
        let stream = timeout(config.connect_timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| RtuError::Timeout)?
            .map_err(RtuError::Io)?;

        configure_socket(&stream, &config)?;

        let framed = Framed::new(stream, RtuOverTcpCodec);
        let (sink, recv_stream) = framed.split();

        Ok((
            RtuTcpSink {
                inner: sink,
                write_timeout: config.write_timeout,
            },
            RtuTcpRecvStream {
                inner: recv_stream,
                read_timeout: config.read_timeout,
            },
        ))
    }
}

/// Apply TCP socket options.
fn configure_socket(stream: &TcpStream, config: &TcpConfig) -> Result<(), RtuError> {
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

/// Write half of an RTU-over-TCP transport.
pub struct RtuTcpSink {
    inner: InnerSink,
    write_timeout: Option<Duration>,
}

impl TransportSink for RtuTcpSink {
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

/// Read half of an RTU-over-TCP transport.
pub struct RtuTcpRecvStream {
    inner: InnerStream,
    read_timeout: Option<Duration>,
}

impl TransportStream for RtuTcpRecvStream {
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
