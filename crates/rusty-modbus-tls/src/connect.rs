//! TLS client and server transport halves.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{Sink, SinkExt, Stream, StreamExt};
use rusty_modbus_frame::error::FrameError;
use rusty_modbus_frame::frame::Frame;
use rusty_modbus_frame::mbap::MbapCodec;
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use tokio_util::codec::Framed;

use crate::config::TlsClientConfig;
use crate::error::TlsError;
use crate::tls_config;

/// TLS client transport — connects to a Modbus/TCP Security server.
pub struct TlsTransport;

impl TlsTransport {
    /// Connect to a Modbus/TCP Security server with mutual TLS authentication.
    ///
    /// Enforces TLS 1.3 with mutual x.509v3 authentication.
    ///
    /// # Errors
    ///
    /// Returns [`TlsError`] on handshake failure, certificate issues, or I/O errors.
    pub async fn connect(
        addr: SocketAddr,
        config: &TlsClientConfig,
    ) -> Result<(TlsSink, TlsRecvStream), TlsError> {
        let rustls_config = tls_config::build_client_config(config)?;
        let connector = TlsConnector::from(Arc::new(rustls_config));

        let tcp_stream = timeout(config.connect_timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| TlsError::Timeout)?
            .map_err(TlsError::Io)?;

        tcp_stream.set_nodelay(true)?;

        let server_name = ServerName::IpAddress(addr.ip().into());
        let tls_stream = connector
            .connect(server_name, tcp_stream)
            .await
            .map_err(|e| TlsError::Handshake(e.to_string()))?;

        let framed = Framed::new(tls_stream, MbapCodec);
        let (sink, stream) = framed.split();

        Ok((
            TlsSink::new(sink, config.write_timeout),
            TlsRecvStream::new(stream, config.read_timeout),
        ))
    }
}

/// Write half of a TLS transport (works for both client and server sides).
pub struct TlsSink {
    inner: Box<dyn Sink<Frame, Error = FrameError> + Send + Unpin>,
    write_timeout: Option<Duration>,
}

impl TlsSink {
    /// Create from any `Sink<Frame>` implementor.
    pub(crate) fn new<S>(sink: S, write_timeout: Option<Duration>) -> Self
    where
        S: Sink<Frame, Error = FrameError> + Send + Unpin + 'static,
    {
        Self {
            inner: Box::new(sink),
            write_timeout,
        }
    }
}

impl rusty_modbus_tcp::TransportSink for TlsSink {
    async fn send(&mut self, frame: Frame) -> Result<(), rusty_modbus_tcp::TransportError> {
        let fut = SinkExt::send(&mut self.inner, frame);
        if let Some(dur) = self.write_timeout {
            timeout(dur, fut)
                .await
                .map_err(|_| rusty_modbus_tcp::TransportError::Timeout)?
                .map_err(rusty_modbus_tcp::TransportError::Frame)
        } else {
            fut.await.map_err(rusty_modbus_tcp::TransportError::Frame)
        }
    }
}

/// Read half of a TLS transport (works for both client and server sides).
pub struct TlsRecvStream {
    inner: Box<dyn Stream<Item = Result<Frame, FrameError>> + Send + Unpin>,
    read_timeout: Option<Duration>,
}

impl TlsRecvStream {
    /// Create from any `Stream<Item = Result<Frame, FrameError>>` implementor.
    pub(crate) fn new<S>(stream: S, read_timeout: Option<Duration>) -> Self
    where
        S: Stream<Item = Result<Frame, FrameError>> + Send + Unpin + 'static,
    {
        Self {
            inner: Box::new(stream),
            read_timeout,
        }
    }
}

impl rusty_modbus_tcp::TransportStream for TlsRecvStream {
    async fn recv(&mut self) -> Result<Frame, rusty_modbus_tcp::TransportError> {
        let fut = self.inner.next();
        let item = if let Some(dur) = self.read_timeout {
            timeout(dur, fut)
                .await
                .map_err(|_| rusty_modbus_tcp::TransportError::Timeout)?
        } else {
            fut.await
        };

        match item {
            Some(Ok(frame)) => Ok(frame),
            Some(Err(e)) => Err(rusty_modbus_tcp::TransportError::Frame(e)),
            None => Err(rusty_modbus_tcp::TransportError::Disconnected),
        }
    }
}
